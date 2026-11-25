//! Inspector form lifetime: forms are keyed by revision and edition so a
//! stale form never submits against a newer project.
mod project;
use crate::backend::{Command, Follow};
use crate::editor::color_dialog::ColourEdit;
use crate::editor::{
    EditorPanel, PanelKind, components, graph_selection, parameter_field, parameter_rows,
    transform_form,
};
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
/// Return the where-used row label shared by the Inspector's list and the
/// delete dialog's description.
pub(in crate::editor) use project::reference_label;
use serde_json::Value;

impl EditorPanel {
    pub(in crate::editor::panels) fn prepare_inspector(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = self.session.read(cx).document();
        if !matches!(self.kind, PanelKind::Inspector) || document.project().is_none() {
            self.value_edit = None;
            return;
        }
        let details = self.session.read(cx).project_inspector.clone();
        let project = details["node"]["project"].clone();
        let project = if project.is_object() {
            project
        } else {
            self.session.read(cx).project_value()
        };

        // The node editor's selected value node: the inline colour page
        // of its owner's parameter — the editor body itself (a
        // Viewer keeps the page; ColourEdit refuses its own edits). The
        // page keys and builds by the LIVE project: the node editor's
        // owner-details ask runs at that same reference, and the
        // Inspector's last item can hold a scene node's older one —
        // a page built on that would refuse its own second edit.
        // Lifecycle: the editor is
        // named by owner and parameter, so an edit's revision refreshes
        // it instead of rebuilding, and it stays mounted while the new
        // owner details travel — typing and focus survive both.
        let target = self.session.read(cx).graph_value();
        let owner_page = target.and_then(|(_, owner, parameter)| {
            // The owner's answer the node editor asked at this very
            // reference; the page keys and builds by the same one.
            let current = self.session.read(cx).project_value();
            let details = self
                .session
                .read(cx)
                .resource_details
                .get(owner)
                .filter(|(read_at, _)| *read_at == current)
                .map(|(_, details)| details.clone())?;
            let row = graph_selection::value_row(&details, parameter)?;
            Some((
                owner.to_owned(),
                parameter.to_owned(),
                current,
                details,
                row,
            ))
        });
        let mounted = self
            .value_edit
            .as_ref()
            .map(|(owner, parameter, _)| (owner.as_str(), parameter.as_str()));
        match graph_selection::value_page_action(
            target.map(|(_, owner, parameter)| (owner, parameter)),
            owner_page.is_some(),
            mounted,
        ) {
            graph_selection::ValuePageAction::Mount => {
                let (owner, parameter, page_project, details, row) =
                    owner_page.expect("mount rides an answered page");
                let target = serde_json::json!({"kind":"resource","id":owner});
                let items = std::rc::Rc::new(details["items"].clone());
                self.value_edit = Some((
                    owner,
                    parameter,
                    cx.new(|cx| {
                        ColourEdit::new(
                            self.session.clone(),
                            target,
                            page_project.clone(),
                            &details,
                            row,
                            items,
                            window,
                            cx,
                        )
                    }),
                ));
            }
            graph_selection::ValuePageAction::Refresh => {
                // The decision sends Refresh only when the mounted
                // editor is this very target's — pin it where the
                // two tuples meet.
                debug_assert_eq!(
                    mounted,
                    owner_page
                        .as_ref()
                        .map(|(owner, parameter, ..)| (owner.as_str(), parameter.as_str())),
                    "refresh keeps the target's own editor mounted"
                );
                let (_, _, page_project, details, row) =
                    owner_page.expect("refresh rides an answered page");
                let scale =
                    parameter_rows::dialog_scale(&parameter_rows::rows(&details["parameters"]));
                let (page_project, row, items) = (
                    page_project.clone(),
                    row.clone(),
                    std::rc::Rc::new(details["items"].clone()),
                );
                if let Some((_, _, edit)) = &self.value_edit {
                    edit.update(cx, |edit, cx| {
                        edit.refresh(&page_project, &row, scale, items, window, cx);
                    });
                }
            }
            // The owner's page details are still in flight for this
            // very target: the mounted editor stays exactly as it is —
            // rebuilding or dropping it would discard the drafts.
            graph_selection::ValuePageAction::Keep => {}
            graph_selection::ValuePageAction::Discard => self.value_edit = None,
        }
        // A Viewer is read-only: drop every edit form so no stale form can
        // submit against it. The value page keeps its editor —
        // ColourEdit refuses its own edits with the Viewer's message.
        if document.is_read_only() {
            self.parameter_forms.clear();
            self.transform_forms.clear();
            self.label_input = None;
            return;
        }

        // Parameters: the node or resource itself, then every component
        // that edits another node.
        // A spectrum data resource's page builds none of
        // them — its Inspector shows the read-only summary instead, so
        // its wavelengths/values tables never become fields.
        let mut pages: Vec<(Value, Value, String)> = Vec::new();
        if details["parameter_target"].is_object()
            && project::item_builds_parameter_rows(&details["item"])
        {
            let edition = details["resource"]["edition"]
                .as_str()
                .or(details["node"]["edition"].as_str())
                .unwrap_or("0")
                .to_owned();
            pages.push((
                details["parameter_target"].clone(),
                details["parameters"].clone(),
                edition,
            ));
        }
        for (id, page) in details["component_parameters"]
            .as_object()
            .into_iter()
            .flatten()
        {
            pages.push((
                serde_json::json!({"kind":"node","id":id}),
                page.clone(),
                project["revision"].as_str().unwrap_or("0").to_owned(),
            ));
        }
        let items = std::rc::Rc::new(details["items"].clone());
        let mut forms = Vec::new();
        for (target, page, edition) in pages {
            let target_id = target["id"].as_str().unwrap_or_default().to_owned();
            // Build one typed field per table row (declared or greyed
            // default) and per declared parameter the table lacks.
            let scale_row = parameter_rows::dialog_scale(&parameter_rows::rows(&page));
            for row in parameter_rows::rows(&page) {
                let key = format!(
                    "{project}:{target}:{edition}:{}:{}:{}",
                    row.value_type, row.name, row.declared
                );
                let form_id = format!("parameter:{target_id}:{}", row.name);
                let reset = self
                    .session
                    .update(cx, |session, _| session.edit_feedback.take_reset(&form_id));
                let existing = self
                    .parameter_forms
                    .iter()
                    .find(|(existing, _, _)| existing == &key && !reset)
                    .map(|(_, _, form)| form.clone());
                if existing.is_none() {
                    // A rebuild of a field that was on screen: tell the user
                    // if it overwrote unsent typing.
                    let previous = self
                        .parameter_forms
                        .iter()
                        .find(|(_, _, form)| form.read(cx).form_id() == form_id)
                        .map(|(_, _, form)| form.read(cx).has_unsent_changes(cx));
                    if let Some(modified) = previous {
                        self.session.update(cx, |session, _| {
                            session.edit_feedback.replaced(&form_id, modified && !reset)
                        });
                    }
                }
                let existing = existing.or_else(|| {
                    let form = self
                        .parameter_forms
                        .iter()
                        .find(|(_, _, form)| form.read(cx).form_id() == form_id)
                        .map(|(_, _, form)| form.clone())?;
                    form.update(cx, |form, cx| {
                        form.refresh_texts(
                            &project,
                            &row,
                            scale_row.clone(),
                            items.clone(),
                            window,
                            cx,
                        )
                    })
                    .then_some(form)
                });
                let form = existing.unwrap_or_else(|| {
                    cx.new(|cx| {
                        parameter_field::ParameterField::new(
                            self.session.clone(),
                            target.clone(),
                            project.clone(),
                            &page,
                            row,
                            items.clone(),
                            window,
                            cx,
                        )
                    })
                });
                forms.push((key, target_id.clone(), form));
            }
        }
        self.parameter_forms = forms;

        // Transform: one local-transform form per node, in the shown mode.
        let mut transforms = Vec::new();
        let node_id = details["node"]["node_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let show_matrix = self.session.read(cx).transform_matrix;
        for component in details["node"]["components"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let local = &component["local"];
            // Camera editability is determined by whether its own Options
            // transform can be represented safely; selection is independent
            // of which Camera is active for rendering.
            let object_editable = match details["node"]["object"].as_str() {
                Some("camera" | "group" | "shape" | "light" | "instance") => true,
                _ => false,
            };
            let editable = object_editable && component["editable"] != false;
            if component["kind"] != "transform" || !local["matrix"].is_array() || !editable {
                continue;
            }
            let matrix = show_matrix || !local["trs"].is_object();
            let key = format!("{project}:{node_id}:{matrix}:{local}");
            let form_id = format!("transform:{node_id}");
            let reset = self
                .session
                .update(cx, |session, _| session.edit_feedback.take_reset(&form_id));
            if let Some(existing) = self
                .transform_forms
                .iter()
                .find(|(k, _, _)| *k == key && !reset)
            {
                transforms.push(existing.clone());
                continue;
            }
            let previous = self
                .transform_forms
                .iter()
                .find(|(_, id, _)| *id == node_id)
                .map(|(_, _, form)| form.read(cx).has_unsent_changes(cx));
            if let Some(modified) = previous {
                self.session.update(cx, |session, _| {
                    session.edit_feedback.replaced(&form_id, modified && !reset)
                });
            }
            if let Some(form) = self
                .transform_forms
                .iter()
                .find(|(_, id, _)| *id == node_id)
                .map(|(_, _, form)| form.clone())
            {
                if form.update(cx, |form, cx| {
                    form.refresh(&project, local, matrix, window, cx)
                }) {
                    transforms.push((key, node_id.clone(), form));
                    continue;
                }
            }
            let form = cx.new(|cx| {
                transform_form::TransformForm::new(
                    self.session.clone(),
                    node_id.clone(),
                    project.clone(),
                    local,
                    matrix,
                    window,
                    cx,
                )
            });
            transforms.push((key, node_id.clone(), form));
        }
        self.transform_forms = transforms;

        let mut animated_transform = None;
        for component in details["node"]["components"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if component["kind"] != "transform" || !component["local"]["samples"].is_object() {
                continue;
            }
            let local = &component["local"];
            let samples = &local["samples"];
            let valid = [&samples["start"], &samples["end"]]
                .into_iter()
                .filter(|sample| sample["matrix"].is_array())
                .collect::<Vec<_>>();
            let has_trs = !valid.is_empty() && valid.iter().all(|sample| sample["trs"].is_object());
            let matrix = show_matrix || !has_trs;
            let key = format!("{project}:{node_id}:{matrix}:{samples}");
            let form = self
                .animated_transform_form
                .as_ref()
                .filter(|(old_key, _)| *old_key == key)
                .map(|(_, form)| form.clone())
                .unwrap_or_else(|| {
                    cx.new(|cx| {
                        transform_form::AnimatedTransformForm::new(samples, matrix, window, cx)
                    })
                });
            animated_transform = Some((key, form));
            break;
        }
        self.animated_transform_form = animated_transform;

        // Label: Enter renames the node or resource.
        let label = details["resource"]["label"]
            .as_str()
            .or(details["node"]["label"].as_str())
            .filter(|_| {
                details["resource"].is_object()
                    || (details["node"].is_object() && details["node"]["data"]["kind"] != "scene")
            });
        let key = label.map(|label| {
            format!(
                "{project}:{}:{}:{label}",
                details["node"]["node_id"], details["resource"]["resource_id"]
            )
        });
        let form_id = label_form_id(&details);
        let reset = self
            .session
            .update(cx, |session, _| session.edit_feedback.take_reset(&form_id));
        match (key, label) {
            (Some(key), Some(label)) => {
                if reset || self.label_input.as_ref().map(|(k, _, _)| k) != Some(&key) {
                    if let Some((_, previous, input)) = &self.label_input {
                        let modified = input.read(cx).value() != previous.as_str();
                        self.session.update(cx, |session, _| {
                            session.edit_feedback.replaced(&form_id, modified && !reset)
                        });
                    }
                    let input = cx.new(|cx| InputState::new(window, cx).default_value(label));
                    let session = self.session.clone();
                    let form = form_id.clone();
                    cx.subscribe(&input, move |_, input, event: &InputEvent, cx| {
                        if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                            return;
                        }
                        let value = input.read(cx).value().to_string();
                        session.update(cx, |session, cx| {
                            let details = session.project_inspector.clone();
                            let project = session.project_value();
                            if let Some(edit) = components::rename_edit(&details, &value) {
                                session.edit_feedback.submitted(&form);
                                session.send(
                                    Command::Apply {
                                        project,
                                        edits: vec![edit],
                                        follow: Follow::Nothing,
                                        form: Some(form.clone()),
                                    },
                                    cx,
                                );
                            }
                        });
                    })
                    .detach();
                    self.label_input = Some((key, label.to_owned(), input));
                }
            }
            _ => self.label_input = None,
        }
    }
}

/// The header label's form id: the resource or node it renames.
pub(super) fn label_form_id(details: &Value) -> String {
    let id = details["resource"]["resource_id"]
        .as_str()
        .or(details["node"]["node_id"].as_str())
        .unwrap_or_default();
    format!("label:{id}")
}

impl EditorPanel {
    pub(in crate::editor::panels) fn inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let document = self.session.read(cx).document();
        let details = if document.project().is_some() {
            self.session.read(cx).project_inspector.clone()
        } else {
            serde_json::json!({"message":"Open a project or a PBRT scene to inspect it"})
        };
        // A node editor's selected value node shows its inline colour
        // page instead of the item target the Inspector last had
        // The page heading shows "owner · parameter", with the colour
        // dialog's content expanded in place.
        let value_target = self
            .session
            .read(cx)
            .graph_value()
            .filter(|_| document.project().is_some())
            .map(|(_, owner, parameter)| (owner.to_owned(), parameter.to_owned()));
        let content = match value_target {
            Some((owner, parameter)) => self.value_node_page(&owner, &parameter, cx),
            None => self.project_inspector(&details, document.is_read_only(), cx),
        };
        div()
            .id("inspector-body")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.inspector_scroll)
            .p_3()
            .text_xs()
            .child(content)
            .into_any_element()
    }

    /// The value node's page: the owning resource and its parameter in
    /// the header, the shared colour editor below — the same fields the
    /// row's colour box opens in a popover, writing the owner's
    /// declaration with the existing `colour_edits`. An eta/k value of a
    /// conductor owner also shows the read-only Metal colour row above
    /// the editor, like the material page does before its eta.
    fn value_node_page(&self, owner: &str, parameter: &str, cx: &mut Context<Self>) -> AnyElement {
        let details = {
            let session = self.session.read(cx);
            let project = session.project_value();
            session
                .resource_details
                .get(owner)
                .filter(|(read_at, _)| *read_at == project)
                .map(|(_, details)| details.clone())
                .unwrap_or_default()
        };
        let label = details["resource"]["label"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| owner.chars().take(8).collect());
        // The Metal colour row answers through the conductor role,
        // whichever metal row the page is; it asks by the owner's own eta.
        let target = serde_json::json!({"kind":"resource","id":owner});
        let metal = match details["resource"]["implementation"].as_str() {
            Some("conductor") if matches!(parameter, "eta" | "k") => Some(
                parameter_field::metal_colour_row(&self.session, &target, "eta", cx),
            ),
            Some("coatedconductor") if matches!(parameter, "conductor.eta" | "conductor.k") => {
                Some(parameter_field::metal_colour_row(
                    &self.session,
                    &target,
                    "conductor.eta",
                    cx,
                ))
            }
            _ => None,
        };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .pb_2()
            .child(Icon::new(assets::IconName::Palette).size(px(18.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(format!("{label} · {parameter}")),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x9098a6))
                            .truncate()
                            .child("spectrum value of the node graph"),
                    ),
            );
        let body = match &self.value_edit {
            Some((_, _, edit)) => div().child(edit.clone()),
            None => div()
                .text_color(rgb(0x9098a6))
                .child("reading the parameter's page…"),
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .children(metal)
            .child(body)
            .into_any_element()
    }
}
