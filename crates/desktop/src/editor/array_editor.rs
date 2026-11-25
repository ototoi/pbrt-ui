//! Dock editor for fixed-size array parameter windows.
use super::{
    Session,
    parameter_values::{self, Table},
};
use crate::backend::Command;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::{
    button::Button,
    input::{Input, InputEvent, InputState},
    pagination::Pagination,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

fn parse_value_window(kind: &str, parsed: Vec<String>) -> Result<Value, String> {
    match kind {
        "floats" => parsed
            .iter()
            .map(|value| {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| "Enter finite numbers".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|value| json!({"kind":"floats","value":value})),
        "integers" => parsed
            .iter()
            .map(|value| {
                value
                    .parse::<i64>()
                    .ok()
                    .filter(|n| n.to_string() == *value)
                    .map(|n| n.to_string())
                    .ok_or_else(|| "Enter signed integer values".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|value| json!({"kind":"integers","value":value})),
        "booleans" => parsed
            .iter()
            .map(|value| {
                value
                    .parse::<bool>()
                    .map_err(|_| "Enter true or false".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|value| json!({"kind":"booleans","value":value})),
        "strings" => Ok(json!({"kind":"strings","value":parsed})),
        "builtin_spectra" => Ok(json!({"kind":"builtin_spectra","value":parsed})),
        "assets" => parsed
            .iter()
            .map(|value| {
                uuid::Uuid::parse_str(value)
                    .map(|id| id.to_string())
                    .map_err(|_| "Enter an asset UUID".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|value| json!({"kind":"assets","value":value})),
        _ => Err("This array value type is read-only".to_owned()),
    }
}

pub struct ArrayEditor {
    session: Entity<Session>,
    target: Value,
    name: String,
    table: Table,
    form: String,
    inputs: Vec<Entity<InputState>>,
    input_key: String,
    cached_payload: Option<Value>,
    loaded_window: Option<(Value, usize)>,
    /// The cells as loaded, to send only what changed.
    loaded_texts: Vec<String>,
    draft_history: bool,
    value_kind: String,
    message: String,
    /// While an edit job runs: the job ids that existed when it was sent.
    following: Option<std::collections::BTreeSet<String>>,
}

/// The cells that differ from what was loaded, as (first index, texts)
/// runs of neighbours. `trim` ignores surrounding spaces (numbers,
/// booleans, ids); string content keeps them. Untouched cells, such as a
/// NaN the editor cannot send, stay out.
pub(super) fn changed_runs(
    loaded: &[String],
    edited: &[String],
    trim: bool,
) -> Vec<(usize, Vec<String>)> {
    let tidy = |text: &str| if trim { text.trim() } else { text }.to_owned();
    let mut runs: Vec<(usize, Vec<String>)> = Vec::new();
    for (index, (before, after)) in loaded.iter().zip(edited).enumerate() {
        if tidy(before) == tidy(after) {
            continue;
        }
        match runs.last_mut() {
            Some((start, texts)) if *start + texts.len() == index => texts.push(tidy(after)),
            _ => runs.push((index, vec![tidy(after)])),
        }
    }
    runs
}

/// The newest `project.set_parameter_values` job for array `key` (core's
/// `target id:name`) not among `known`: (state, error message).
pub(super) fn array_job(
    jobs: &Value,
    key: &str,
    known: &std::collections::BTreeSet<String>,
) -> Option<(String, Option<String>)> {
    jobs["jobs"]
        .as_array()?
        .iter()
        .rev()
        .find(|job| {
            job["kind"] == "project.set_parameter_values"
                && job["root_path"] == key
                && job["job_id"].as_str().is_some_and(|id| !known.contains(id))
        })
        .map(|job| {
            (
                job["state"].as_str().unwrap_or_default().to_owned(),
                job["error"]["message"].as_str().map(str::to_owned),
            )
        })
}

impl ArrayEditor {
    pub fn new(
        session: Entity<Session>,
        entry: &Value,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let target = entry["target"].clone();
        let name = entry["name"].as_str().unwrap_or_default().to_owned();
        let table = Table::from_value(&entry["table"]).expect("array editor table");
        let form = format!(
            "parameter:{}:{}",
            target["id"].as_str().unwrap_or_default(),
            name
        );
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            target,
            name,
            table,
            form,
            inputs: Vec::new(),
            input_key: String::new(),
            cached_payload: None,
            loaded_window: None,
            loaded_texts: Vec::new(),
            draft_history: false,
            value_kind: String::new(),
            message: String::new(),
            following: None,
        }
    }

    fn refresh_inputs(
        &mut self,
        payload: &Value,
        project: &Value,
        page: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let values = &payload["values"];
        let kind = values["kind"].as_str().unwrap_or_default().to_owned();
        let key = format!("{project}:{page}:{values}");
        if key == self.input_key {
            return;
        }
        let incoming = values["value"]
            .as_array()
            .into_iter()
            .flatten()
            .map(parameter_values::cell_text)
            .collect::<Vec<_>>();
        let reuse = self
            .loaded_window
            .as_ref()
            .is_some_and(|(_, previous)| *previous == page)
            && self.value_kind == kind
            && incoming.len() == self.inputs.len();
        self.loaded_window = Some((project.clone(), page));
        self.input_key = key;
        self.cached_payload = Some(payload.clone());
        self.value_kind = kind;
        self.draft_history = false;
        if reuse {
            for (input, text) in self.inputs.iter().zip(&incoming) {
                input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
            }
            self.loaded_texts = incoming;
            return;
        }
        self.inputs.clear();
        self.loaded_texts.clear();
        for value in values["value"].as_array().into_iter().flatten() {
            let text = parameter_values::cell_text(value);
            self.loaded_texts.push(text.clone());
            let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.draft_history = true;
                }
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.apply_window(cx);
                }
            })
            .detach();
            cx.observe(&input, |_, _, cx| cx.notify()).detach();
            self.inputs.push(input);
        }
    }

    fn apply_window(&mut self, cx: &mut Context<Self>) {
        let current = self.session.read(cx);
        let expected = (
            current.project_value(),
            current.array_page_number(&self.form),
        );
        if self.loaded_window.as_ref() != Some(&expected) {
            self.message = "Wait for the selected page to load".into();
            cx.notify();
            return;
        }
        if self.session.read(cx).document().is_read_only() {
            self.message = "Viewer is read-only".into();
            cx.notify();
            return;
        }
        let (offset, _) = self
            .table
            .window(self.session.read(cx).array_page_number(&self.form));
        if self.following.is_some() {
            self.message = "Applying the previous change…".into();
            cx.notify();
            return;
        }
        let edited = self
            .inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect::<Vec<_>>();
        // Only the changed cells go out, so an untouched value the editor
        // cannot send never blocks the rest of the page.
        let trim = !matches!(self.value_kind.as_str(), "strings" | "builtin_spectra");
        let runs = changed_runs(&self.loaded_texts, &edited, trim);
        if runs.is_empty() {
            self.message = "No changes".into();
            cx.notify();
            return;
        }
        let mut windows = Vec::with_capacity(runs.len());
        for (start, texts) in runs {
            match parse_value_window(&self.value_kind, texts) {
                Ok(values) => windows.push(json!({"offset":offset + start,"values":values})),
                Err(error) => {
                    self.message = error;
                    cx.notify();
                    return;
                }
            }
        }
        let project = self.session.read(cx).project_value();
        if !project.is_object() {
            self.message = "No editable project is open".into();
            cx.notify();
            return;
        }
        // A job: a data asset is rewritten on core's I/O executor. Follow
        // it from the job list.
        let params = json!({"project":project,"target":self.target,"name":self.name,
            "windows":windows});
        self.following = Some(
            self.session.read(cx).jobs["jobs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|job| job["job_id"].as_str().map(str::to_owned))
                .collect(),
        );
        let form = self.form.clone();
        self.session.update(cx, |session, cx| {
            session.edit_feedback.submitted(&form);
            session.send(
                Command::Submit {
                    method: protocol::Method::ProjectSetParameterValues,
                    params,
                    form,
                },
                cx,
            );
        });
        self.message.clear();
        cx.notify();
    }
}

impl Render for ArrayEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let project = self.session.read(cx).project_value();
        let read_only = self.session.read(cx).document().is_read_only();
        let page = self.session.update(cx, |session, cx| {
            session.array_page(&self.form, &self.target, &self.name, &self.table, cx)
        });
        let (group_page, payload) = page.unzip();
        let group_page = group_page.unwrap_or(0);
        if let Some(payload) = payload.as_ref() {
            if let Some(total) = payload
                .get("total")
                .and_then(Value::as_str)
                .and_then(|v| v.parse::<u64>().ok())
            {
                self.table.count = total;
            } else if payload.get("error").is_some() && group_page > 0 {
                self.session
                    .update(cx, |session, cx| session.array_reset_page(&self.form, cx));
            }
        }
        if let Some(payload) = payload
            .as_ref()
            .filter(|payload| payload.get("error").is_none())
        {
            self.refresh_inputs(payload, &project, group_page, window, cx);
        }
        let payload = payload.or_else(|| {
            self.loaded_window
                .as_ref()
                .filter(|(_, page)| *page == group_page)
                .and(self.cached_payload.clone())
        });
        let header = div()
            .flex()
            .gap_2()
            .items_center()
            .text_xs()
            .text_color(rgb(0x858b97))
            .child(format!(
                "{} · {} · {} values · {}",
                self.name,
                self.table.value_type,
                self.table.count,
                if read_only {
                    "read-only Viewer"
                } else {
                    "edits keep the array length"
                }
            ));
        let content = match payload {
            None => div().child("Reading values…"),
            Some(payload) if payload.get("error").is_some() => {
                div().text_color(rgb(0xd99a74)).child(
                    payload["error"]
                        .as_str()
                        .unwrap_or("Values could not be read")
                        .to_owned(),
                )
            }
            Some(_) => {
                let first = group_page * parameter_values::PAGE_GROUPS;
                let width = self.table.width.max(1);
                let labels = match (self.table.value_type.as_str(), width) {
                    ("point3" | "vector3" | "normal3", 3) => vec!["#", "x", "y", "z"],
                    ("point2" | "vector2", 2) => vec!["#", "x", "y"],
                    ("rgb" | "spectrum", 3) => vec!["#", "r", "g", "b"],
                    _ => std::iter::once("#")
                        .chain(std::iter::repeat_n("value", width))
                        .collect(),
                };
                let grid_header = div()
                    .flex()
                    .h(px(22.))
                    .bg(rgb(0x292c32))
                    .border_b_1()
                    .border_color(rgb(0x414650))
                    .children(labels.into_iter().enumerate().map(|(column, label)| {
                        div()
                            .when(column == 0, |cell| cell.w(px(40.)).flex_shrink_0())
                            .when(column != 0, |cell| cell.flex_1().min_w_0())
                            .px_1()
                            .border_r_1()
                            .border_color(rgb(0x414650))
                            .text_xs()
                            .text_color(rgb(0xabb2bf))
                            .child(label.to_owned())
                    }));
                let grid =
                    div().children(self.inputs.chunks(width).enumerate().map(|(row, cells)| {
                        div()
                            .flex()
                            .h(px(22.))
                            .border_b_1()
                            .border_color(rgb(0x363b43))
                            .bg(if row % 2 == 0 {
                                rgb(0x1c1f24)
                            } else {
                                rgb(0x202329)
                            })
                            .child(
                                div()
                                    .w(px(40.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .px_1()
                                    .bg(rgb(0x292c32))
                                    .border_r_1()
                                    .border_color(rgb(0x414650))
                                    .text_xs()
                                    .text_color(rgb(0x8d95a3))
                                    .child((first + row).to_string()),
                            )
                            .children(cells.iter().map(|input| {
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .border_r_1()
                                    .border_color(rgb(0x363b43))
                                    .child(
                                        Input::new(input)
                                            .xsmall()
                                            .appearance(false)
                                            .readonly(read_only)
                                            .rounded_none()
                                            .h(px(21.))
                                            .px_1(),
                                    )
                            }))
                    }));
                let pager = (self.table.pages() > 1).then(|| {
                    let (session, form) = (self.session.clone(), self.form.clone());
                    Pagination::new(format!("{}-array-editor-pages", self.form))
                        .current_page(group_page + 1)
                        .total_pages(self.table.pages() as usize)
                        .compact()
                        .on_click(move |page, _, cx| {
                            session.update(cx, |session, cx| {
                                session.array_show_page(&form, page.saturating_sub(1), cx)
                            })
                        })
                });
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(rgb(0x414650))
                    .child(grid_header)
                    .child(
                        div()
                            .id("array-editor-grid")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(grid),
                    )
                    .when_some(pager, |this, pager| this.child(pager))
            }
        };
        // The job this editor sent: "Applying…" until it settles, then its
        // failure or nothing.
        // Refused before it became a job: no job will
        // come, so stop waiting; the refusal shows as the form's feedback.
        if self.following.is_some()
            && self.session.update(cx, |session, _| {
                session.edit_feedback.take_reset(&self.form)
            })
        {
            self.following = None;
            self.message.clear();
        }
        if let Some(known) = &self.following {
            let key = format!(
                "{}:{}",
                self.target["id"].as_str().unwrap_or_default(),
                self.name
            );
            match array_job(&self.session.read(cx).jobs, &key, known) {
                Some((state, error)) if matches!(state.as_str(), "failed" | "cancelled") => {
                    self.message = error.unwrap_or_else(|| format!("Edit {state}"));
                    self.following = None;
                }
                Some((state, _)) if state == "succeeded" => {
                    self.message.clear();
                    self.following = None;
                }
                _ => self.message = "Applying…".into(),
            }
        }
        let ready = self.following.is_none()
            && self.loaded_window.as_ref()
                == Some(&(
                    project.clone(),
                    self.session.read(cx).array_page_number(&self.form),
                ));
        let apply = cx.listener(|this, _, _, cx| this.apply_window(cx));
        div()
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Undo, window, cx| {
                    if !this.draft_history
                        && true
                        && this.following.is_none()
                        && !this.session.read(cx).edit_feedback.is_pending(&this.form)
                    {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(super::menu::UndoEdit), cx);
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Redo, window, cx| {
                    if !this.draft_history
                        && true
                        && this.following.is_none()
                        && !this.session.read(cx).edit_feedback.is_pending(&this.form)
                    {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(super::menu::RedoEdit), cx);
                    }
                },
            ))
            .track_focus(&cx.focus_handle())
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .child(header)
            .child(content)
            .when(!read_only, |this| {
                this.child(
                    Button::new("apply-array-window")
                        .label("Apply Page")
                        .disabled(!ready)
                        .small()
                        .on_click(apply),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0xb8bbc4))
                    .child(if self.message.is_empty() {
                        super::parameter_form::feedback_text("", &self.session, &self.form, cx)
                    } else {
                        self.message.clone()
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{array_job, changed_runs, parse_value_window};
    use serde_json::json;

    /// Only the cells that differ from what was loaded are sent, as runs of
    /// neighbours; an untouched NaN cell is never part of one.
    #[test]
    fn changed_cells_become_runs_and_untouched_cells_stay_out() {
        let text = |cells: &[&str]| cells.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        let loaded = text(&["1.0", "nan", "3.0", "4.0", "5.0", "6.0"]);
        assert!(changed_runs(&loaded, &loaded, true).is_empty());
        let edited = text(&["1.0", "nan", "3.5", "4.5", "5.0", "6.5"]);
        assert_eq!(
            changed_runs(&loaded, &edited, true),
            [(2, text(&["3.5", "4.5"])), (5, text(&["6.5"]))]
        );
        // Whitespace around a number is not a change.
        let padded = text(&["1.0", "nan", " 3.0 ", "4.0", "5.0", "6.0"]);
        assert!(changed_runs(&loaded, &padded, true).is_empty());
    }

    /// In a string array whitespace is content: a padded name is a change
    /// and goes out exactly as typed.
    #[test]
    fn string_cells_keep_their_whitespace() {
        let loaded = vec!["name".to_owned(), "other".to_owned()];
        let edited = vec![" name ".to_owned(), "other".to_owned()];
        assert_eq!(
            changed_runs(&loaded, &edited, false),
            [(0, vec![" name ".to_owned()])]
        );
    }

    /// The editor follows the job it sent: the newest set_parameter_values
    /// job for this array that was not there when it sent.
    #[test]
    fn the_editor_follows_its_own_job() {
        let jobs = json!({"jobs":[
            {"job_id":"old","kind":"project.set_parameter_values","root_path":"n:P","state":"failed",
             "error":{"message":"earlier"}},
            {"job_id":"other","kind":"project.set_parameter_values","root_path":"n:N","state":"running"},
            {"job_id":"mine","kind":"project.set_parameter_values","root_path":"n:P","state":"failed",
             "error":{"message":"project revision changed"}},
        ]});
        let known = ["old".to_owned()].into_iter().collect();
        assert_eq!(
            array_job(&jobs, "n:P", &known),
            Some((
                "failed".to_owned(),
                Some("project revision changed".to_owned())
            ))
        );
        let all = ["old".to_owned(), "mine".to_owned()].into_iter().collect();
        assert_eq!(array_job(&jobs, "n:P", &all), None);
    }

    #[test]
    fn array_editor_preserves_integer_precision_and_rejects_non_finite_numbers() {
        assert_eq!(
            parse_value_window("integers", vec!["9007199254740993".into()]).unwrap(),
            json!({"kind":"integers","value":["9007199254740993"]})
        );
        assert!(parse_value_window("floats", vec!["NaN".into()]).is_err());
        assert!(parse_value_window("integers", vec!["01".into()]).is_err());
    }

    #[test]
    fn array_editor_serializes_boolean_and_string_windows_without_changing_the_kind() {
        assert_eq!(
            parse_value_window("booleans", vec!["true".into(), "false".into()]).unwrap(),
            json!({"kind":"booleans","value":[true,false]})
        );
        assert_eq!(
            parse_value_window("strings", vec!["one".into(), "two".into()]).unwrap(),
            json!({"kind":"strings","value":["one","two"]})
        );
    }
}
