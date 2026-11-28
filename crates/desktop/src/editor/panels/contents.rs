//! Contents panel: the `Assets/` folder tree and the Built-in presets.
use crate::backend::{Command, Follow, PreviewKind};
use crate::editor::EditorPanel;
use crate::editor::contents_tree::{self, BUILTIN, PAGE_LIMIT, Row};
use crate::editor::menu::{
    ContentsDelete, ContentsImportResource, ContentsMove, ContentsNewFolder, ContentsOpen,
    ContentsRename, CreatePreset, CreateResource, NewScene, OpenInEditor,
};
use crate::editor::resource_menu;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu};
use gpui_kit::component::{Icon, Sizable, button::Button, spinner::Spinner};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ContentsPane {
    Tree,
    Grid,
}

const GRID_CELL_WIDTH: f32 = 120.;
const GRID_ROW_HEIGHT: f32 = 124.;
const GRID_GAP: f32 = 8.;
const GRID_MAX_COLUMNS: usize = 8;
use crate::editor::project_contents_search::SEARCH_PAGE_LIMIT;
const SEARCH_MORE: &str = "@contents-search";

impl EditorPanel {
    pub(super) fn contents_search_changed(&mut self, cx: &mut Context<Self>) {
        let Some(input) = &self.contents_search_input else {
            return;
        };
        let query = input.read(cx).value().trim().to_owned();
        self.contents_search_generation = self.contents_search_generation.wrapping_add(1);
        self.contents_search_more_offset = None;
        let generation = self.contents_search_generation;
        let project = self.session.read(cx).project_value();
        self.session.update(cx, |session, cx| {
            session.contents_search_query = query.clone();
            session.contents_search = Value::Null;
            session.contents_search_project = project.clone();
            session.contents_search_generation = generation;
            session.contents_search_error = None;
            session.contents_search_in_flight = None;
            session.contents_search_settled = None;
            cx.notify();
        });
        if query.is_empty() {
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.contents_search_generation == generation
                    && this
                        .contents_search_input
                        .as_ref()
                        .is_some_and(|input| input.read(cx).value().trim() == query)
                {
                    this.send_contents_search(project, query, generation, 0, cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn contents_search_submit(&mut self, cx: &mut Context<Self>) {
        let Some(input) = &self.contents_search_input else {
            return;
        };
        let query = input.read(cx).value().trim().to_owned();
        self.contents_search_generation = self.contents_search_generation.wrapping_add(1);
        self.contents_search_more_offset = None;
        let generation = self.contents_search_generation;
        let project = self.session.read(cx).project_value();
        self.session.update(cx, |session, cx| {
            session.contents_search_query = query.clone();
            session.contents_search = Value::Null;
            session.contents_search_project = project.clone();
            session.contents_search_generation = generation;
            session.contents_search_error = None;
            session.contents_search_in_flight = None;
            session.contents_search_settled = None;
            cx.notify();
        });
        if !query.is_empty() {
            self.send_contents_search(project, query, generation, 0, cx);
        }
        cx.notify();
    }

    fn send_contents_search(
        &self,
        project: Value,
        query: String,
        generation: u64,
        offset: usize,
        cx: &mut Context<Self>,
    ) {
        self.session.update(cx, |session, cx| {
            session.request_contents_search(
                project,
                query,
                generation,
                offset,
                SEARCH_PAGE_LIMIT,
                cx,
            );
        });
    }

    fn clear_contents_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.contents_search_generation = self.contents_search_generation.wrapping_add(1);
        self.contents_search_more_offset = None;
        let generation = self.contents_search_generation;
        self.session.update(cx, |session, cx| {
            session.contents_search_query.clear();
            session.contents_search = Value::Null;
            session.contents_search_project = Value::Null;
            session.contents_search_generation = generation;
            session.contents_search_error = None;
            session.contents_search_in_flight = None;
            session.contents_search_settled = None;
            cx.notify();
        });
        if let Some(input) = &self.contents_search_input {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
    }

    pub(super) fn contents(&self, cx: &mut Context<Self>) -> AnyElement {
        let model = self.session.read(cx);
        let project = model.project_value();
        let read_only = model.document().is_read_only();
        let tree_rows = model.contents.rows();
        let query = self
            .contents_search_input
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        let search_mode = !query.is_empty();
        let search_page = &model.contents_search;
        let folder_rows = if search_mode {
            let mut rows = search_page["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned()
                .map(|entry| Row::Entry { entry, depth: 0 })
                .collect::<Vec<_>>();
            if let Some(offset) = search_page["next_offset"].as_u64() {
                rows.push(Row::More {
                    folder: SEARCH_MORE.to_owned(),
                    offset: offset as usize,
                    depth: 0,
                });
            }
            rows
        } else {
            model.contents.folder_rows()
        };
        let current_folder = model.contents.current_folder().to_owned();
        let folder_loading = model.contents.folder_is_loading(&project);
        let search_matches = search_page["total"].as_u64();
        let search_ready = search_page.is_object()
            && search_page["project"] == project
            && search_page["query"] == query;
        let search_requested =
            model.contents_search_query == query && model.contents_search_project == project;
        let search_error = model.contents_search_error.clone();
        let search_loading =
            search_mode && search_requested && !search_ready && search_error.is_none();
        let grid_columns = self.contents_grid_columns.max(1);
        let list_generation = (
            model.contents.generation(),
            model.contents_search_generation,
        );
        let resource_schema = model.resource_schema.clone();
        let thumb_rows = thumbnail_candidate_rows(&folder_rows);
        let panel = cx.entity();
        let row_project = project.clone();
        let row_data = tree_rows.clone();
        let row_count = tree_rows.len();
        let tree_list = uniform_list(
            "contents-tree-list",
            row_count,
            move |range, _window, app| {
                range
                    .filter_map(|index| {
                        let row = row_data.get(index)?.clone();
                        Some(panel.update(app, |this, cx| {
                            this.contents_row(
                                index,
                                row,
                                &row_project,
                                read_only,
                                ContentsPane::Tree,
                                false,
                                cx,
                            )
                        }))
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.contents_tree_scroll)
        .h_full()
        .min_h_0();
        let panel = cx.entity();
        let row_project = project.clone();
        let row_data = folder_rows.clone();
        let row_count = folder_rows.len().div_ceil(grid_columns);
        let panel_for_grid = cx.entity();
        let folder_list = uniform_list(
            "contents-folder-grid",
            row_count,
            move |range, _window, app| {
                range
                    .filter_map(|index| {
                        let start = index * grid_columns;
                        let end = (start + grid_columns).min(row_data.len());
                        let rows = row_data.get(start..end)?.to_vec();
                        Some(panel.update(app, |this, cx| {
                            this.contents_grid_row(
                                start,
                                rows,
                                &row_project,
                                read_only,
                                search_mode,
                                cx,
                            )
                        }))
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.contents_list_scroll)
        .h_full()
        .with_decoration(ContentsVisibleDecoration {
            session: self.session.downgrade(),
            panel: panel_for_grid.downgrade(),
            project: project.clone(),
            candidates: thumb_rows,
            list_generation,
            columns: grid_columns,
        })
        .min_h_0();
        if !project.is_object() {
            return div()
                .px_3()
                .py_2()
                .text_xs()
                .text_color(rgb(0x8d93a1))
                .child("Open a project folder or a PBRT scene to see its contents.")
                .into_any_element();
        }

        let can_edit = !read_only;
        let session = self.session.clone();
        let refresh_project = project.clone();
        div()
            .id("contents-body")
            .size_full()
            .flex()
            .flex_col()
            .child(
                div().px_2().py_1().child(
                    Input::new(self.contents_search_input.as_ref().unwrap())
                        .small()
                        .cleanable(true)
                        .w_full(),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(0x8d93a1))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .child(if current_folder == BUILTIN {
                                "Built-in".to_owned()
                            } else if current_folder.is_empty() {
                                "Assets/".to_owned()
                            } else {
                                format!("Assets/{current_folder}/")
                            }),
                    )
                    .when(can_edit, |bar| {
                        bar.child(
                            Button::new("contents-new-folder")
                                .icon(assets::IconName::FolderPlus)
                                .small()
                                .tooltip("New Folder")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ContentsNewFolder), cx)
                                }),
                        )
                        .child(
                            Button::new("contents-import-resource")
                                .icon(assets::IconName::Download)
                                .small()
                                .tooltip("Import image, PLY or SPD")
                                .accessibility_label("Import an image, PLY or SPD file")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ContentsImportResource), cx)
                                }),
                        )
                        .child(
                            Button::new("contents-new-resource")
                                .icon(assets::IconName::Plus)
                                .small()
                                .tooltip("New resource")
                                .accessibility_label("Create a resource")
                                .dropdown_menu(move |menu, window, cx| {
                                    resource_kind_menus(menu, window, cx, &resource_schema)
                                }),
                        )
                    })
                    .child(
                        Button::new("contents-refresh")
                            .icon(assets::IconName::RefreshCw)
                            .small()
                            .tooltip("Refresh contents")
                            .accessibility_label("Refresh project contents")
                            .on_click(move |_, _, cx| {
                                session.update(cx, |session, cx| {
                                    session.contents_thumbnails.refresh();
                                    session.backend.update_thumbnail_visibility(
                                        session.project_value(),
                                        session.contents_thumbnails.generation(),
                                        Vec::new(),
                                    );
                                    session.clear_thumbnail_images();
                                    let folders = session.contents.stale_folders(&Value::Null);
                                    for folder in folders {
                                        session.send(
                                            Command::ProjectContents {
                                                project: refresh_project.clone(),
                                                folder,
                                                offset: 0,
                                                limit: PAGE_LIMIT,
                                            },
                                            cx,
                                        );
                                    }
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        div()
                            .w(px(220.))
                            .min_w(px(128.))
                            .max_w(px(280.))
                            .h_full()
                            .min_h_0()
                            .border_r_1()
                            .border_color(rgb(0x353943))
                            .child(tree_list),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .min_h_0()
                            .when(search_mode, |pane| {
                                pane.child(
                                    div()
                                        .px_3()
                                        .py_1()
                                        .text_xs()
                                        .text_color(rgb(0x8d93a1))
                                        .child(if search_loading {
                                            format!("Searching for {query}…")
                                        } else if let Some(error) = search_error.clone() {
                                            format!("Search failed: {error}")
                                        } else if !search_requested {
                                            "Project changed; press Enter to search again"
                                                .to_owned()
                                        } else if let Some(total) = search_matches {
                                            format!("{total} results for {query}")
                                        } else {
                                            format!("Search results for {query}")
                                        }),
                                )
                            })
                            .when(folder_rows.is_empty(), |pane| {
                                pane.child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .text_xs()
                                        .text_color(rgb(0x8d93a1))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        // Non-blocking status: a small
                                        // spinner beside the text.
                                        .when(folder_loading && !search_mode, |row| {
                                            row.child(Spinner::new().xsmall())
                                        })
                                        .child(if search_mode && search_loading {
                                            "Searching…"
                                        } else if search_mode && search_error.is_some() {
                                            "Search failed"
                                        } else if search_mode && !search_requested {
                                            "Press Enter to search this project"
                                        } else if search_mode {
                                            "No matching items"
                                        } else if folder_loading {
                                            "Loading folder…"
                                        } else {
                                            "This folder is empty"
                                        }),
                                )
                            })
                            .child(folder_list),
                    ),
            )
            .into_any_element()
    }

    /// Editable rows can be dragged onto a folder to move them there.
    fn draggable(&self, row: Stateful<Div>, entry: &Value, editable: bool) -> Stateful<Div> {
        if !editable {
            return row;
        }
        let name = entry["name"].as_str().unwrap_or_default().to_owned();
        row.on_drag(
            ContentsDrag {
                entry: entry.clone(),
            },
            move |_, _, _, cx| cx.new(|_| DragLabel(name.clone())),
        )
    }

    /// Folder rows (and the `Assets` root) take dropped rows in a Project.
    fn drop_target(
        &self,
        row: Stateful<Div>,
        folder: &str,
        read_only: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        if read_only || folder == BUILTIN {
            return row;
        }
        let accepts = folder.to_owned();
        let target = folder.to_owned();
        row.can_drop(move |value, _, _| {
            value
                .downcast_ref::<ContentsDrag>()
                .is_some_and(|drag| contents_tree::drop_edit(&drag.entry, &accepts).is_some())
        })
        .drag_over::<ContentsDrag>(|style, _, _, _| style.bg(rgb(0x2f4a3a)))
        .on_drop(cx.listener(move |this, drag: &ContentsDrag, _, cx| {
            let Some(edit) = contents_tree::drop_edit(&drag.entry, &target) else {
                return;
            };
            this.session.update(cx, |session, cx| {
                let project = session.project_value();
                session.contents.select(None);
                session.send(
                    Command::Apply {
                        project,
                        edits: vec![edit],
                        follow: Follow::Nothing,
                        form: None,
                    },
                    cx,
                );
            });
        }))
    }

    fn contents_grid_row(
        &self,
        start: usize,
        rows: Vec<Row>,
        project: &Value,
        read_only: bool,
        search_mode: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut grid = div()
            .id(("contents-grid-row", start))
            .flex()
            .items_start()
            .gap(px(GRID_GAP))
            .h(px(GRID_ROW_HEIGHT))
            .w_full();
        for (offset, row) in rows.into_iter().enumerate() {
            grid = grid.child(self.contents_row(
                start + offset,
                row,
                project,
                read_only,
                ContentsPane::Grid,
                search_mode,
                cx,
            ));
        }
        grid.into_any_element()
    }

    // One call site renders a row from its separate pieces; a struct would only repack them.
    #[allow(clippy::too_many_arguments)]
    fn contents_row(
        &self,
        index: usize,
        row: Row,
        project: &Value,
        read_only: bool,
        pane: ContentsPane,
        search_mode: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // r4's static creation rows: the menu reads the cached
        // summary, fetched once at startup.
        let resource_schema = self.session.read(cx).resource_schema.clone();
        let identity = row_identity(&row, index);
        let base = |depth: usize| {
            let row = div().id(("contents-row", stable_id(&identity)));
            if pane == ContentsPane::Grid {
                row.w(px(GRID_CELL_WIDTH))
                    .h(px(GRID_ROW_HEIGHT))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .p_1()
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
            } else {
                row.flex()
                    .items_center()
                    .gap_1()
                    .mx_1()
                    .pl(px(6. + depth as f32 * 14.))
                    .pr_2()
                    .h(px(24.))
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
            }
        };
        match row {
            Row::Folder {
                path,
                name,
                depth,
                expanded,
                loading,
            } => {
                let entry = serde_json::json!({"kind":"folder","name":name,"location":path});
                let contents = &self.session.read(cx).contents;
                let selected = contents.is_selected(&entry)
                    || (pane == ContentsPane::Tree && contents.current_folder() == path);
                let builtin = path == BUILTIN;
                let editable = !read_only && !builtin && !path.is_empty();
                let disclosure = if pane == ContentsPane::Tree {
                    let path = path.clone();
                    let project = project.clone();
                    div()
                        .id(("contents-disclosure", stable_id(&path)))
                        .w(px(14.))
                        .h(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .child(
                            Icon::new(if expanded {
                                assets::IconName::ChevronDown
                            } else {
                                assets::IconName::ChevronRight
                            })
                            .size(px(12.)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let (path, project) = (path.clone(), project.clone());
                            this.session.update(cx, |session, cx| {
                                if session.contents.toggle(&path, &project) {
                                    session.send(
                                        Command::ProjectContents {
                                            project,
                                            folder: path,
                                            offset: 0,
                                            limit: PAGE_LIMIT,
                                        },
                                        cx,
                                    );
                                }
                                cx.notify();
                            });
                            cx.stop_propagation();
                        }))
                } else {
                    div()
                        .id(("contents-disclosure-empty", stable_id(&path)))
                        .w(px(if pane == ContentsPane::Grid { 1. } else { 14. }))
                };
                let row_project = project.clone();
                let row_path = path.clone();
                let row_entry = entry.clone();
                let row = base(depth)
                    .when(selected, |row| row.bg(rgb(0x3a3d45)))
                    .hover(|style| style.bg(rgb(0x30333a)))
                    .child(disclosure)
                    .child(
                        Icon::new(if builtin {
                            assets::IconName::Package
                        } else if expanded {
                            assets::IconName::FolderOpen
                        } else {
                            assets::IconName::Folder
                        })
                        .size(px(if pane == ContentsPane::Grid { 48. } else { 14. }))
                        .text_color(rgb(0xc7b37a)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .w_full()
                            .truncate()
                            .when(pane == ContentsPane::Grid, |label| label.text_center())
                            .child(name),
                    )
                    .when(loading, |row| {
                        row.child(div().text_xs().text_color(rgb(0x8d93a1)).child("…"))
                    })
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        let project = row_project.clone();
                        let path = row_path.clone();
                        let entry = row_entry.clone();
                        if pane == ContentsPane::Grid && event.click_count() >= 2 && search_mode {
                            this.clear_contents_search(window, cx);
                        }
                        this.session.update(cx, |session, cx| {
                            let selected = (!builtin && !path.is_empty()).then_some(entry);
                            if pane == ContentsPane::Tree {
                                session.contents.select(selected);
                                if let Some(folder) =
                                    session.contents.select_folder(&path, &project)
                                {
                                    session.send(
                                        Command::ProjectContents {
                                            project,
                                            folder,
                                            offset: 0,
                                            limit: PAGE_LIMIT,
                                        },
                                        cx,
                                    );
                                }
                            } else if event.click_count() >= 2 {
                                session.contents.select(selected);
                                for folder in session.contents.open_folder(&path, &project) {
                                    session.send(
                                        Command::ProjectContents {
                                            project: project.clone(),
                                            folder,
                                            offset: 0,
                                            limit: PAGE_LIMIT,
                                        },
                                        cx,
                                    );
                                }
                            } else {
                                session.contents.select(selected);
                                for folder in session.contents.reveal(&path, &project) {
                                    session.send(
                                        Command::ProjectContents {
                                            project: project.clone(),
                                            folder,
                                            offset: 0,
                                            limit: PAGE_LIMIT,
                                        },
                                        cx,
                                    );
                                }
                            }
                            cx.notify();
                        });
                    }));
                let row =
                    self.drop_target(self.draggable(row, &entry, editable), &path, read_only, cx);
                if !editable {
                    return row.into_any_element();
                }
                let context_path = path.clone();
                let context_project = project.clone();
                let context_entry = entry.clone();
                row.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _, _, cx| {
                        let (entry, path, project) = (
                            context_entry.clone(),
                            context_path.clone(),
                            context_project.clone(),
                        );
                        this.session.update(cx, |session, cx| {
                            session.contents.select(Some(entry));
                            let folders = if pane == ContentsPane::Tree {
                                session
                                    .contents
                                    .select_folder(&path, &project)
                                    .into_iter()
                                    .collect()
                            } else {
                                session.contents.reveal(&path, &project)
                            };
                            for folder in folders {
                                session.send(
                                    Command::ProjectContents {
                                        project: project.clone(),
                                        folder,
                                        offset: 0,
                                        limit: PAGE_LIMIT,
                                    },
                                    cx,
                                );
                            }
                            cx.notify();
                        });
                    }),
                )
                .context_menu(move |menu, window, cx| {
                    let schema = resource_schema.clone();
                    new_resource_menu(menu, window, cx, &schema)
                        .menu("Import Resource…", Box::new(ContentsImportResource))
                        .separator()
                        .menu("Rename…", Box::new(ContentsRename))
                        .menu("Move…", Box::new(ContentsMove))
                        .menu("Delete…", Box::new(ContentsDelete))
                })
                .into_any_element()
            }
            Row::More {
                folder,
                offset,
                depth,
            } => {
                let project = project.clone();
                let search_more_loading =
                    folder == SEARCH_MORE && self.contents_search_more_offset == Some(offset);
                base(depth)
                    .text_xs()
                    .text_color(rgb(0x8d93a1))
                    .hover(|style| style.bg(rgb(0x30333a)))
                    .child(if search_more_loading {
                        "Loading…"
                    } else {
                        "… load more"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let (project, folder) = (project.clone(), folder.clone());
                        let search_more = folder == SEARCH_MORE;
                        if search_more {
                            if this.contents_search_more_offset == Some(offset) {
                                return;
                            }
                            this.contents_search_more_offset = Some(offset);
                        }
                        this.session.update(cx, |session, cx| {
                            if folder == SEARCH_MORE {
                                let query = session.contents_search_query.clone();
                                let project = session.contents_search_project.clone();
                                let generation = session.contents_search_generation;
                                if !query.is_empty()
                                    && project == session.project_value()
                                    && session.contents_search["next_offset"].as_u64()
                                        == Some(offset as u64)
                                    && session.contents_search["query"] == query
                                    && session.contents_search["project"] == project
                                {
                                    session.request_contents_search(
                                        project,
                                        query,
                                        generation,
                                        offset,
                                        SEARCH_PAGE_LIMIT,
                                        cx,
                                    );
                                }
                            } else {
                                session.send(
                                    Command::ProjectContents {
                                        project,
                                        folder,
                                        offset,
                                        limit: PAGE_LIMIT,
                                    },
                                    cx,
                                )
                            }
                        });
                        if search_more {
                            let session = this.session.read(cx);
                            if session.contents_search_in_flight
                                != Some((session.contents_search_generation, offset))
                            {
                                this.contents_search_more_offset = None;
                            }
                        }
                    }))
                    .into_any_element()
            }
            Row::Entry { entry, depth } => {
                let selected = self.session.read(cx).contents.is_selected(&entry);
                let builtin = entry["builtin"] == true;
                let scene = entry["kind"] == "scene";
                let active_scene = scene
                    && self
                        .session
                        .read(cx)
                        .project
                        .active_scene
                        .map(|id| id.to_string())
                        == entry["id"].as_str().map(str::to_owned);
                let detail = contents_detail(&entry);
                let detail = if search_mode {
                    let location = entry["location"].as_str().unwrap_or_default();
                    let folder = if entry["builtin"] == true {
                        "Built-in".to_owned()
                    } else {
                        let parent = contents_tree::parent(location);
                        if parent.is_empty() {
                            "Assets/".to_owned()
                        } else {
                            format!("Assets/{parent}/")
                        }
                    };
                    format!("{detail} · {folder}")
                } else {
                    detail
                };
                let thumbnail = (pane == ContentsPane::Grid)
                    .then(|| {
                        self.session
                            .update(cx, |session, _| session.thumbnail_image(&entry))
                    })
                    .flatten();
                // A Spectrum Data row's grid thumbnail shows the
                // curve's chromaticity colour (the same swatch the node
                // editor and the Spectrum Data editor show), replacing
                // This replaces the small curve thumbnail: the colour says what the curve is;
                // the editor holds the curve itself.
                let spectrum_colour = (pane == ContentsPane::Grid
                    && entry["kind"] == "resource"
                    && entry["resource_kind"] == "spectrum_data")
                    .then(|| {
                        self.session.update(cx, |session, cx| {
                            session.spectrum_swatch(
                                entry["id"].as_str().unwrap_or_default(),
                                "chromaticity",
                                cx,
                            )
                        })
                    })
                    .flatten()
                    .and_then(|answer| swatch_answer_colour(&answer));
                let project = project.clone();
                let click_entry = entry.clone();
                let draggable = !read_only && contents_tree::editable(&entry);
                let mut row = base(depth)
                    .when(selected, |row| row.bg(rgb(0x3a3d45)))
                    .hover(|style| style.bg(rgb(0x30333a)))
                    .when(pane == ContentsPane::Tree, |row| {
                        row.child(div().w(px(12.)))
                    })
                    .child(if let Some(image) = thumbnail {
                        img(image)
                            .size(px(if pane == ContentsPane::Grid { 64. } else { 20. }))
                            .rounded_sm()
                            .into_any_element()
                    } else if let Some(colour) = spectrum_colour {
                        div()
                            .size(px(64.))
                            .rounded_sm()
                            .bg(colour)
                            .into_any_element()
                    } else {
                        Icon::new(contents_icon(&entry))
                            .size(px(if pane == ContentsPane::Grid { 48. } else { 14. }))
                            .text_color(rgb(0xaeb5c4))
                            .into_any_element()
                    })
                    .child(
                        div()
                            .flex_1()
                            .w_full()
                            .truncate()
                            .when(pane == ContentsPane::Grid, |label| label.text_center())
                            .when(active_scene, |label| label.font_weight(FontWeight::BOLD))
                            .when(entry["kind"] == "unregistered", |label| {
                                label.text_color(rgb(0x8d93a1))
                            })
                            .child(entry["name"].as_str().unwrap_or_default().to_owned()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x8d93a1))
                            .when(pane == ContentsPane::Grid, |label| label.truncate())
                            .child(detail),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        let entry = click_entry.clone();
                        let project = project.clone();
                        if event.click_count() >= 2 {
                            if search_mode && entry["kind"] == "folder" {
                                this.clear_contents_search(window, cx);
                                let location =
                                    entry["location"].as_str().unwrap_or_default().to_owned();
                                this.session.update(cx, |session, cx| {
                                    session.contents.select(Some(entry));
                                    for folder in session.contents.open_folder(&location, &project)
                                    {
                                        session.send(
                                            Command::ProjectContents {
                                                project: project.clone(),
                                                folder,
                                                offset: 0,
                                                limit: PAGE_LIMIT,
                                            },
                                            cx,
                                        );
                                    }
                                    cx.notify();
                                });
                            } else if entry["kind"] == "scene" {
                                window.dispatch_action(Box::new(ContentsOpen), cx);
                            } else if let Some(id) = entry["id"].as_str().filter(|_| {
                                crate::editor::editor_panels::editor_for(&entry).is_some()
                            }) {
                                window.dispatch_action(
                                    Box::new(OpenInEditor { id: id.to_owned() }),
                                    cx,
                                );
                            } else if entry["builtin"] == true
                                && let Some(preset) = entry["id"]
                                    .as_str()
                                    .and_then(|id| uuid::Uuid::parse_str(id).ok())
                            {
                                window.dispatch_action(Box::new(CreatePreset { preset }), cx);
                            }
                            return;
                        }
                        this.session.update(cx, |session, cx| {
                            let folders = if entry["builtin"] == true {
                                session.contents.open_folder(BUILTIN, &project)
                            } else {
                                session.contents.reveal(
                                    entry["location"].as_str().unwrap_or_default(),
                                    &project,
                                )
                            };
                            for folder in folders {
                                session.send(
                                    Command::ProjectContents {
                                        project: project.clone(),
                                        folder,
                                        offset: 0,
                                        limit: PAGE_LIMIT,
                                    },
                                    cx,
                                );
                            }
                            session.select_contents(entry, project, cx);
                        });
                    }));
                row = self.draggable(row, &entry, draggable);
                if (!read_only && contents_tree::editable(&entry)) || scene || builtin {
                    let menu_entry = entry.clone();
                    let project = self.session.read(cx).project_value();
                    row = row.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            let (entry, project) = (menu_entry.clone(), project.clone());
                            this.session.update(cx, |session, cx| {
                                let folders = if entry["builtin"] == true {
                                    session.contents.open_folder(BUILTIN, &project)
                                } else {
                                    session.contents.reveal(
                                        entry["location"].as_str().unwrap_or_default(),
                                        &project,
                                    )
                                };
                                for folder in folders {
                                    session.send(
                                        Command::ProjectContents {
                                            project: project.clone(),
                                            folder,
                                            offset: 0,
                                            limit: PAGE_LIMIT,
                                        },
                                        cx,
                                    );
                                }
                                session.select_contents(entry, project, cx);
                            });
                        }),
                    );
                    let editable = !read_only && contents_tree::editable(&entry);
                    let preset = entry["id"]
                        .as_str()
                        .and_then(|id| uuid::Uuid::parse_str(id).ok())
                        .filter(|_| builtin && !read_only);
                    let editor_id = crate::editor::editor_panels::editor_for(&entry)
                        .and(entry["id"].as_str())
                        .map(str::to_owned);
                    return row
                        .context_menu(move |menu, _, _| {
                            let mut menu = menu;
                            if scene {
                                menu = menu.menu("Open Scene", Box::new(ContentsOpen));
                            }
                            if let Some(id) = &editor_id {
                                menu = menu.menu(
                                    "Open in Editor",
                                    Box::new(OpenInEditor { id: id.clone() }),
                                );
                            }
                            if let Some(preset) = preset {
                                menu = menu
                                    .menu("Create in Hierarchy", Box::new(CreatePreset { preset }));
                            }
                            if editable {
                                menu = menu
                                    .separator()
                                    .menu("New Folder", Box::new(ContentsNewFolder))
                                    .menu("Import Resource…", Box::new(ContentsImportResource))
                                    .menu("Rename…", Box::new(ContentsRename))
                                    .menu("Move…", Box::new(ContentsMove))
                                    .menu("Delete…", Box::new(ContentsDelete));
                            }
                            menu
                        })
                        .into_any_element();
                }
                row.into_any_element()
            }
        }
    }
}

struct ContentsVisibleDecoration {
    session: WeakEntity<crate::editor::Session>,
    panel: WeakEntity<EditorPanel>,
    project: Value,
    candidates: Vec<Option<(String, PreviewKind)>>,
    list_generation: (u64, u64),
    columns: usize,
}

impl UniformListDecoration for ContentsVisibleDecoration {
    fn compute(
        &self,
        visible_range: std::ops::Range<usize>,
        bounds: Bounds<Pixels>,
        _: Point<Pixels>,
        _: Pixels,
        _: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let candidates = visible_grid_thumbnail_candidates(
            &self.candidates,
            visible_range,
            self.columns,
            bounds.size.height > px(0.),
        );
        let columns = grid_columns_for_width(bounds.size.width);
        let project = self.project.clone();
        let list_generation = self.list_generation;
        let session = self.session.clone();
        let panel = self.panel.clone();
        cx.defer(move |cx| {
            let _ = panel.update(cx, |panel, cx| {
                if panel.contents_grid_columns != columns {
                    panel.contents_grid_columns = columns;
                    cx.notify();
                }
            });
            let _ = session.update(cx, |session, cx| {
                session.contents_visible(project, list_generation, candidates, cx)
            });
        });
        div().into_any_element()
    }
}

/// Return which folder rows may request a thumbnail as
/// `(id, kind)` — textures, materials and image assets. Everything else
/// (PLY and other non-image assets, unregistered files, scenes, folders,
/// page markers) stays `None`. The list keeps the rows' order so the grid
/// can slice visible ranges out of it.
fn thumbnail_candidate_rows(rows: &[Row]) -> Vec<Option<(String, PreviewKind)>> {
    rows.iter()
        .map(|row| match row {
            Row::Entry { entry, .. } => PreviewKind::of_entry(entry)
                .zip(entry["id"].as_str())
                .map(|(kind, id)| (id.to_owned(), kind)),
            _ => None,
        })
        .collect()
}

fn visible_grid_thumbnail_candidates(
    candidates: &[Option<(String, PreviewKind)>],
    visible_rows: std::ops::Range<usize>,
    columns: usize,
    has_viewport: bool,
) -> Vec<(String, PreviewKind)> {
    if !has_viewport || columns == 0 {
        return Vec::new();
    }
    visible_rows
        .flat_map(|row| {
            let start = row.saturating_mul(columns);
            let end = (start + columns).min(candidates.len());
            start..end
        })
        .filter_map(|index| candidates.get(index).and_then(Clone::clone))
        .collect()
}

fn grid_columns_for_width(width: Pixels) -> usize {
    ((width / px(GRID_CELL_WIDTH + GRID_GAP)).floor() as usize).clamp(1, GRID_MAX_COLUMNS)
}

fn row_identity(row: &Row, index: usize) -> String {
    match row {
        Row::Folder { path, .. } => format!("folder:{path}"),
        Row::Entry { entry, .. } => entry["id"]
            .as_str()
            .map(|id| {
                if entry["builtin"] == true {
                    format!("entry:builtin:{id}")
                } else {
                    format!("entry:{id}")
                }
            })
            .or_else(|| {
                entry["location"]
                    .as_str()
                    .map(|path| format!("entry:{path}"))
            })
            .unwrap_or_else(|| format!("entry-index:{index}")),
        Row::More { folder, offset, .. } => format!("more:{folder}:{offset}"),
    }
}

fn stable_id(value: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hash);
    hash.finish()
}

/// Return the RGB paint for a spectrum swatch answer, or None while it
/// stays muted — on its way, refused, or empty.
fn swatch_answer_colour(answer: &Value) -> Option<Rgba> {
    if answer.get("error").is_some() {
        return None;
    }
    let channel = |index: usize| {
        answer["rgb"]
            .as_array()
            .and_then(|values| values.get(index))
            .and_then(Value::as_f64)
            .unwrap_or_default()
            .clamp(0., 1.)
    };
    Some(Rgba {
        r: channel(0) as f32,
        g: channel(1) as f32,
        b: channel(2) as f32,
        a: 1.,
    })
}

/// Icon per Contents entry, from its kind and resource kind.
fn contents_icon(entry: &Value) -> assets::IconName {
    use assets::IconName;
    match (entry["kind"].as_str(), entry["resource_kind"].as_str()) {
        (Some("scene"), _) => IconName::Clapperboard,
        (_, Some("material")) => IconName::Palette,
        (_, Some("texture")) => IconName::Image,
        (_, Some("medium")) => IconName::Droplets,
        (_, Some("spectrum_data")) => IconName::ChartSpline,
        (_, Some("object_definition")) => IconName::Boxes,
        (_, Some("preset")) => IconName::SquarePlus,
        (Some("asset"), _) => match entry["media_type"].as_str().unwrap_or("") {
            media if media.starts_with("image/") => IconName::Image,
            media if media.contains("ply") || media.contains("mesh") => IconName::Triangle,
            _ => IconName::FileBox,
        },
        _ => IconName::FileBox,
    }
}

/// Right-hand text: what kind of item, and how many objects use it.
fn contents_detail(entry: &Value) -> String {
    let kind = match entry["kind"].as_str().unwrap_or("") {
        "resource" => match entry["resource_kind"].as_str().unwrap_or("resource") {
            // The one kind spelled with an underscore on the wire.
            "spectrum_data" => "spectrum data",
            other => other,
        },
        "unregistered" => "not registered",
        other => other,
    };
    match entry["reference_count"].as_u64() {
        Some(count) if count > 0 => format!("{kind} · {count}"),
        _ => kind.to_owned(),
    }
}

#[cfg(test)]
mod spectrum_kind_tests {
    // Specific imports: a glob here chains through gpui-kit's exports and
    // shadows `#[test]`.
    use super::{contents_detail, contents_icon};

    #[test]
    fn spectrum_data_rows_carry_their_own_icon_and_detail() {
        let entry = serde_json::json!({"kind":"resource","resource_kind":"spectrum_data",
            "id":"00000000-0000-0000-0000-0000000000aa","name":"Au.k"});
        assert_eq!(
            contents_icon(&entry),
            gpui_kit::assets::IconName::ChartSpline
        );
        assert_eq!(contents_detail(&entry), "spectrum data");
        let used = serde_json::json!({"kind":"resource","resource_kind":"spectrum_data",
            "reference_count":2});
        assert_eq!(contents_detail(&used), "spectrum data · 2");
    }
}

/// What a Contents row carries while it is dragged.
struct ContentsDrag {
    entry: Value,
}

/// The label that follows the pointer during a drag.
struct DragLabel(String);

impl Render for DragLabel {
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

/// The rows of one implementation list: a `CreateResource` action each.
fn implementation_rows(
    menu: PopupMenu,
    kind: &str,
    names: &[String],
    texture_value_type: Option<&str>,
) -> PopupMenu {
    let mut menu = menu;
    for name in names {
        menu = menu.menu(
            name.clone(),
            Box::new(CreateResource {
                kind: kind.to_owned(),
                implementation: name.clone(),
                texture_value_type: texture_value_type.map(str::to_owned),
            }),
        );
    }
    menu
}

/// The New ▸ rows: Material ▸ implementations,
/// Texture ▸ Float | Spectrum ▸ implementations, Medium ▸ implementations,
/// one Object Definition entry, and one Spectrum Data entry. The
/// core fills the flat 400–700 nm unit curve the kind starts as). The
/// implementations come straight from r4's static parameter table;
/// object definitions and spectrum data have no table rows, so each gets
/// a single entry.
fn resource_kind_menus(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
    schema: &Value,
) -> PopupMenu {
    let material = resource_menu::implementations(&schema["material"]);
    let texture_float = resource_menu::implementations(&schema["texture_float"]);
    let texture_spectrum = resource_menu::implementations(&schema["texture_spectrum"]);
    let medium = resource_menu::implementations(&schema["medium"]);
    let rows = material.clone();
    let menu = menu.submenu("Material", window, cx, move |menu, _, _| {
        implementation_rows(menu, "material", &rows, None)
    });
    let (float, spectrum) = (texture_float.clone(), texture_spectrum.clone());
    let menu = menu.submenu("Texture", window, cx, move |menu, window, cx| {
        let rows = float.clone();
        let menu = menu.submenu("Float", window, cx, move |menu, _, _| {
            implementation_rows(menu, "texture", &rows, Some("float"))
        });
        let rows = spectrum.clone();
        menu.submenu("Spectrum", window, cx, move |menu, _, _| {
            implementation_rows(menu, "texture", &rows, Some("spectrum"))
        })
    });
    let rows = medium.clone();
    let menu = menu.submenu("Medium", window, cx, move |menu, _, _| {
        implementation_rows(menu, "medium", &rows, None)
    });
    menu.menu(
        "Object Definition",
        Box::new(CreateResource {
            kind: "object_definition".into(),
            implementation: String::new(),
            texture_value_type: None,
        }),
    )
    .menu(
        "Spectrum Data",
        Box::new(CreateResource {
            kind: "spectrum_data".into(),
            implementation: "piecewise".into(),
            texture_value_type: None,
        }),
    )
}

/// The folder-row right-click menu opens the kinds under a `New` submenu;
/// the toolbar's + button shows them one level up. New Folder and New Scene
/// sit in the same submenu.
fn new_resource_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
    schema: &Value,
) -> PopupMenu {
    let schema = schema.clone();
    menu.submenu("New", window, cx, move |menu, window, cx| {
        resource_kind_menus(menu, window, cx, &schema)
            .separator()
            .menu("Folder", Box::new(ContentsNewFolder))
            .menu("Scene", Box::new(NewScene))
    })
}

#[cfg(test)]
mod thumbnail_visibility_tests {
    use super::{
        grid_columns_for_width, px, thumbnail_candidate_rows, visible_grid_thumbnail_candidates,
    };
    use crate::backend::PreviewKind;
    use crate::editor::contents_tree::Row;
    use serde_json::json;

    #[test]
    fn grid_columns_adapt_to_the_available_width() {
        assert_eq!(grid_columns_for_width(px(40.)), 1);
        assert_eq!(grid_columns_for_width(px(250.)), 1);
        assert_eq!(grid_columns_for_width(px(300.)), 2);
        assert_eq!(grid_columns_for_width(px(520.)), 4);
        assert_eq!(grid_columns_for_width(px(2_000.)), 8);
    }

    /// Image assets join textures and materials as thumbnail
    /// candidates; PLY assets, unregistered files, scenes, folders and page
    /// markers stay icons.
    #[test]
    fn thumbnail_candidates_cover_image_assets_and_skip_everything_else() {
        let rows = vec![
            Row::Folder {
                path: "Images".into(),
                name: "Images".into(),
                depth: 1,
                expanded: false,
                loading: false,
            },
            Row::Entry {
                entry: json!({"kind":"asset","media_type":"image/png","id":"png"}),
                depth: 2,
            },
            Row::Entry {
                entry: json!({"kind":"asset","media_type":"image/x-exr","id":"exr"}),
                depth: 2,
            },
            Row::Entry {
                entry: json!({"kind":"asset","media_type":"application/x-ply","id":"ply"}),
                depth: 2,
            },
            Row::Entry {
                entry: json!({"kind":"resource","resource_kind":"texture","id":"tex"}),
                depth: 2,
            },
            Row::Entry {
                entry: json!({"kind":"resource","resource_kind":"material","id":"mat"}),
                depth: 2,
            },
            Row::Entry {
                entry: json!({"kind":"unregistered","id":"fresh"}),
                depth: 2,
            },
            Row::Entry {
                entry: json!({"kind":"scene","id":"scene"}),
                depth: 2,
            },
            Row::More {
                folder: "Images".into(),
                offset: 256,
                depth: 2,
            },
        ];
        assert_eq!(
            thumbnail_candidate_rows(&rows),
            vec![
                None,
                Some(("png".into(), PreviewKind::ImageAsset)),
                Some(("exr".into(), PreviewKind::ImageAsset)),
                None,
                Some(("tex".into(), PreviewKind::Texture)),
                Some(("mat".into(), PreviewKind::Material)),
                None,
                None,
                None,
            ]
        );
    }

    #[test]
    fn thumbnail_requests_follow_only_visible_grid_rows() {
        let cells = (0..10)
            .map(|index| {
                Some((
                    format!("item-{index}"),
                    if index % 2 == 0 {
                        PreviewKind::ImageAsset
                    } else {
                        PreviewKind::Texture
                    },
                ))
            })
            .collect::<Vec<_>>();
        assert!(visible_grid_thumbnail_candidates(&cells, 0..3, 4, false).is_empty());
        assert_eq!(
            visible_grid_thumbnail_candidates(&cells, 1..2, 4, true),
            vec![
                ("item-4".into(), PreviewKind::ImageAsset),
                ("item-5".into(), PreviewKind::Texture),
                ("item-6".into(), PreviewKind::ImageAsset),
                ("item-7".into(), PreviewKind::Texture),
            ]
        );
        assert_eq!(
            visible_grid_thumbnail_candidates(&cells, 2..3, 4, true),
            vec![
                ("item-8".into(), PreviewKind::ImageAsset),
                ("item-9".into(), PreviewKind::Texture)
            ]
        );
        assert!(visible_grid_thumbnail_candidates(&cells, 0..3, 0, true).is_empty());
    }
}
