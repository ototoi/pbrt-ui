//! Revision-checked editor for a node's local transform: TRS
//! or the 4×4 matrix, sent as one
//! `set_local_trs` / `set_local_matrix`.
use super::Session;
use crate::backend::{Command, Follow};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::{
        Sizable,
        input::{Input, InputEvent, InputState},
    },
    *,
};
use serde_json::{Value, json};

pub struct TransformForm {
    session: Entity<Session>,
    node_id: String,
    project: Value,
    matrix: bool,
    inputs: Vec<Entity<InputState>>,
    /// The exact values and the text shown for them when the form was built:
    /// a field left as shown sends its exact value, not the rounded text.
    initial: Vec<(f64, String)>,
    draft_history: bool,
    submitted_texts: Option<Vec<String>>,
    message: String,
}

/// Read-only paired endpoint fields for an AnimatedTransform.
pub struct AnimatedTransformForm {
    rows: Vec<(String, Vec<Entity<InputState>>, Vec<Entity<InputState>>)>,
    matrix: bool,
}

impl AnimatedTransformForm {
    pub fn new(samples: &Value, matrix: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let endpoint_rows = |sample: &Value| -> Vec<(String, Vec<String>)> {
            if !sample["matrix"].is_array() {
                let reason = crate::editor::components::transform_reason(&sample["matrix_reason"])
                    .map(|reason| format!("Unavailable: {reason}"))
                    .unwrap_or_else(|| "Unavailable".into());
                return vec![("Status".into(), vec![reason])];
            }
            if matrix {
                let values = sample["matrix"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|value| number_text(value.as_f64().unwrap_or(f64::NAN)))
                    .collect::<Vec<_>>();
                let mut rows = values
                    .chunks(4)
                    .enumerate()
                    .map(|(index, values)| (format!("Row {index}"), values.to_vec()))
                    .collect::<Vec<_>>();
                if let Some(reason) =
                    crate::editor::components::transform_reason(&sample["trs_reason"])
                {
                    rows.push((
                        "Status".into(),
                        vec![format!("TRS unavailable: matrix {reason}")],
                    ));
                }
                rows
            } else {
                [
                    ("Position", "translation"),
                    ("Rotation", "rotation_euler_degrees"),
                    ("Scale", "scale"),
                ]
                .into_iter()
                .map(|(label, key)| {
                    let values = sample["trs"][key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|value| number_text(value.as_f64().unwrap_or(f64::NAN)))
                        .collect();
                    (label.to_owned(), values)
                })
                .collect()
            }
        };
        let left_rows = endpoint_rows(&samples["start"]);
        let right_rows = endpoint_rows(&samples["end"]);
        let labels = if matrix {
            vec!["Row 0", "Row 1", "Row 2", "Row 3", "Status"]
        } else {
            vec!["Position", "Rotation", "Scale", "Status"]
        };
        let rows = labels
            .into_iter()
            .filter_map(|label| {
                let left = left_rows
                    .iter()
                    .find(|(name, _)| name == label)
                    .map(|(_, values)| values.clone())
                    .unwrap_or_default();
                let right = right_rows
                    .iter()
                    .find(|(name, _)| name == label)
                    .map(|(_, values)| values.clone())
                    .unwrap_or_default();
                (!left.is_empty() || !right.is_empty()).then(|| {
                    let inputs =
                        |values: Vec<String>, window: &mut Window, cx: &mut Context<Self>| {
                            values
                                .into_iter()
                                .map(|text| {
                                    let input = cx
                                        .new(|cx| InputState::new(window, cx).default_value(text));
                                    cx.observe(&input, |_, _, cx| cx.notify()).detach();
                                    input
                                })
                                .collect()
                        };
                    (
                        label.to_owned(),
                        inputs(left, window, cx),
                        inputs(right, window, cx),
                    )
                })
            })
            .collect();
        Self { rows, matrix }
    }
}

impl Render for AnimatedTransformForm {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut form = div().flex().flex_col().gap_1();
        for (label, start, end) in &self.rows {
            let fields = |inputs: &[Entity<InputState>]| {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .w_full()
                    .children(inputs.iter().map(|input| {
                        let field = Input::new(input).small().readonly(true).w_full();
                        let field = if self.matrix && label.starts_with("Row ") {
                            field.font_family("DejaVu Sans Mono").whitespace_nowrap()
                        } else {
                            field
                        };
                        div().flex_1().min_w_0().child(field)
                    }))
            };
            form = form.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py(px(2.))
                    .child(
                        div()
                            .w(px(92.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(0x9098a6))
                            .child(label.clone()),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            div()
                                .flex()
                                .gap_2()
                                .child(div().flex_1().min_w_0().child(fields(start)))
                                .child(div().flex_1().min_w_0().child(fields(end))),
                        ),
                    ),
            );
        }
        form
    }
}

/// Six decimals, without trailing zeros or negative zero.
pub fn number_text(v: f64) -> String {
    let v = (v * 1e6).round() / 1e6 + 0.;
    let text = format!("{v:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The values a local shows in the form: TRS's nine or the matrix's sixteen.
fn values(local: &Value, matrix: bool) -> Vec<f64> {
    let numbers = |value: &Value| -> Vec<f64> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_f64().unwrap_or(f64::NAN))
            .collect()
    };
    if matrix {
        numbers(&local["matrix"])
    } else {
        ["translation", "rotation_euler_degrees", "scale"]
            .iter()
            .flat_map(|key| numbers(&local["trs"][key]))
            .collect()
    }
}

/// The decision Enter's apply and the "Enter to apply" hint share:
/// whether any typed field left the text the form was built with. The
/// rounded initial text decides, not the exact value behind it — a
/// number typed differently counts until apply parses it back.
fn modified(texts: &[String], initial: &[(f64, String)]) -> bool {
    texts
        .iter()
        .zip(initial)
        .any(|(typed, (_, shown))| typed != shown)
}

/// Whether typed values sit unsent — the Transform box's ⋮ Apply
/// Transform enable state. Values matching the texts just submitted
/// count as sent: no second send until the project answers and the
/// form refreshes (Enter applies).
fn has_unsent_edits(
    texts: &[String],
    initial: &[(f64, String)],
    submitted: Option<&[String]>,
) -> bool {
    modified(texts, initial) && submitted != Some(texts)
}

/// The edit for `values`, or why it cannot be sent.
fn edit(node_id: &str, matrix: bool, values: Vec<f64>) -> Result<Value, String> {
    if matrix {
        let matrix: [f64; 16] = values
            .try_into()
            .map_err(|_| "Expected sixteen values".to_owned())?;
        Ok(json!({"kind":"set_local_matrix","node_id":node_id,"matrix":matrix}))
    } else {
        let v: [f64; 9] = values
            .try_into()
            .map_err(|_| "Expected nine values".to_owned())?;
        Ok(json!({
            "kind":"set_local_trs","node_id":node_id,
            "translation":[v[0], v[1], v[2]],
            "rotation_euler_degrees":[v[3], v[4], v[5]],
            "scale":[v[6], v[7], v[8]],
        }))
    }
}

impl TransformForm {
    pub fn new(
        session: Entity<Session>,
        node_id: String,
        project: Value,
        local: &Value,
        matrix: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial: Vec<(f64, String)> = values(local, matrix)
            .into_iter()
            .map(|v| (v, number_text(v)))
            .collect();
        let inputs = initial
            .iter()
            .map(|(_, text)| cx.new(|cx| InputState::new(window, cx).default_value(text.clone())))
            .collect::<Vec<_>>();
        for input in &inputs {
            cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.draft_history = true;
                }
                // Enter applies every field as the one edit the Apply
                // Transform button sends — one set_local_trs /
                // set_local_matrix, one Undo step. Blur never applies:
                // tabbing between fields must not stack undo steps.
                if matches!(event, InputEvent::PressEnter { .. }) && this.is_modified(cx) {
                    this.apply(cx);
                }
            })
            .detach();
            cx.observe(input, |_, _, cx| cx.notify()).detach();
        }
        Self {
            session,
            node_id,
            project,
            matrix,
            inputs,
            initial,
            draft_history: false,
            submitted_texts: None,
            message: String::new(),
        }
    }

    pub fn refresh(
        &mut self,
        project: &Value,
        local: &Value,
        matrix: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.matrix != matrix {
            return false;
        }
        let initial = values(local, matrix)
            .into_iter()
            .map(|v| (v, number_text(v)))
            .collect::<Vec<_>>();
        if initial.len() != self.inputs.len() {
            return false;
        }
        for (input, (_, text)) in self.inputs.iter().zip(&initial) {
            input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
        }
        self.project = project.clone();
        self.initial = initial;
        self.draft_history = false;
        self.submitted_texts = None;
        self.message.clear();
        cx.notify();
        true
    }

    /// Stable across revisions: `transform:<node id>`.
    pub fn form_id(&self) -> String {
        format!("transform:{}", self.node_id)
    }

    /// The text every field holds, the way apply reads it.
    fn typed_texts(&self, cx: &App) -> Vec<String> {
        self.inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect()
    }

    /// The user changed a field and did not send it — the ⋮ menu's
    /// Whether Apply Transform is enabled.
    pub fn has_unsent_changes(&self, cx: &App) -> bool {
        let texts = self.typed_texts(cx);
        has_unsent_edits(&texts, &self.initial, self.submitted_texts.as_deref())
    }

    pub fn is_modified(&self, cx: &App) -> bool {
        modified(&self.typed_texts(cx), &self.initial)
    }

    /// The ⋮ menu's Apply Transform calls this — the same send Enter
    /// makes: one set_local_trs / set_local_matrix, one Undo step.
    pub(super) fn apply(&mut self, cx: &mut Context<Self>) {
        let values = self
            .inputs
            .iter()
            .zip(&self.initial)
            .map(|(input, (exact, text))| {
                let typed = input.read(cx).value().to_string();
                if typed == *text {
                    return Ok(*exact);
                }
                typed
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| "Enter finite numbers in every transform field".to_owned())
            })
            .collect::<Result<Vec<_>, _>>();
        let edit = match values.and_then(|values| edit(&self.node_id, self.matrix, values)) {
            Ok(edit) => edit,
            Err(error) => {
                self.message = error;
                cx.notify();
                return;
            }
        };
        let project = self.session.read(cx).project_value();
        if project != self.project || !project.is_object() {
            self.message =
                "Project changed; wait for the Inspector to refresh before applying".into();
            cx.notify();
            return;
        }
        self.submitted_texts = Some(
            self.inputs
                .iter()
                .map(|input| input.read(cx).value().to_string())
                .collect(),
        );
        let form = self.form_id();
        self.session.update(cx, |session, cx| {
            session.edit_feedback.submitted(&form);
            session.send(
                Command::Apply {
                    project,
                    edits: vec![edit],
                    follow: Follow::Nothing,
                    form: Some(form.clone()),
                },
                cx,
            )
        });
        self.message.clear();
        cx.notify();
    }
}

impl Render for TransformForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut form = div()
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Undo, window, cx| {
                    if !this.draft_history
                        && !this.is_modified(cx)
                        && !this
                            .session
                            .read(cx)
                            .edit_feedback
                            .is_pending(&this.form_id())
                    {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(super::menu::UndoEdit), cx);
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Redo, window, cx| {
                    if !this.draft_history
                        && !this.is_modified(cx)
                        && !this
                            .session
                            .read(cx)
                            .edit_feedback
                            .is_pending(&this.form_id())
                    {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(super::menu::RedoEdit), cx);
                    }
                },
            ))
            .flex()
            .flex_col()
            .gap_1();
        let rows: Vec<(&str, usize)> = if self.matrix {
            vec![("Row 0", 4), ("Row 1", 4), ("Row 2", 4), ("Row 3", 4)]
        } else {
            vec![("Position", 3), ("Rotation", 3), ("Scale", 3)]
        };
        let mut inputs = self.inputs.iter();
        for (label, count) in rows {
            // The value area shares the row with the label — the same
            // shape the sample display above uses — so the fields fit
            // the Inspector instead of running past its right edge;
            // one row is three columns, a matrix four.
            let mut values = div().flex().items_center().gap_1().flex_1().min_w_0();
            for input in inputs.by_ref().take(count) {
                values = values.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(input).xsmall().w_full()),
                );
            }
            // A narrow label column and xsmall fields leave each value room for
            // its digits (`0.470123`) at the default Inspector width.
            let line = div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .w(px(56.))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(0x9098a6))
                        .child(label),
                )
                .child(values);
            form = form.child(line);
        }
        let feedback =
            super::parameter_form::feedback_text(&self.message, &self.session, &self.form_id(), cx);
        // The apply button is gone: Enter applies, and the
        // component header's ⋮ menu carries Apply Transform for the
        // mouse path, enabled while edits are pending.
        form
            // While a field is off its initial text, the same hint the
            // parameter fields show beside their own values.
            .when(self.is_modified(cx) && feedback.is_empty(), |form| {
                form.child(
                    div()
                        .pl(px(60.))
                        .text_xs()
                        .text_color(rgb(0x9098a6))
                        .child("Enter to apply"),
                )
            })
            .child(div().text_xs().text_color(rgb(0xb8bbc4)).child(feedback))
    }
}

#[cfg(test)]
mod tests {
    use super::{edit, has_unsent_edits, modified, number_text};
    use serde_json::json;

    #[test]
    fn number_text_trims_and_drops_negative_zero() {
        assert_eq!(number_text(30.000000000000004), "30");
        assert_eq!(number_text(-0.0000001), "0");
        assert_eq!(number_text(0.125), "0.125");
        assert_eq!(number_text(-2.5), "-2.5");
    }

    #[test]
    fn only_fields_off_their_initial_texts_are_modified() {
        let initial = vec![(1., "1".to_owned()), (2.5, "2.5".to_owned())];
        // The texts the form was built with are not a modification.
        assert!(!modified(&["1".to_owned(), "2.5".to_owned()], &initial));
        // One changed field is.
        assert!(modified(&["1".to_owned(), "3".to_owned()], &initial));
        // A differently formatted value counts too: apply parses the
        // typed text back, it does not read the rounded original.
        assert!(modified(&["1.0".to_owned(), "2.5".to_owned()], &initial));
    }

    #[test]
    fn edits_carry_the_values_in_wire_order() {
        let trs = edit("n", false, (1..=9).map(f64::from).collect()).unwrap();
        assert_eq!(trs["rotation_euler_degrees"], json!([4., 5., 6.]));
        assert_eq!(trs["kind"], "set_local_trs");
        let matrix = edit("n", true, vec![0.; 16]).unwrap();
        assert_eq!(matrix["kind"], "set_local_matrix");
        assert!(edit("n", true, vec![0.; 9]).is_err());
    }

    /// The Transform component's ⋮ Apply Transform is enabled only
    /// while typed values sit unsent (Enter applies).
    #[test]
    fn apply_is_enabled_only_while_edits_are_pending() {
        let initial = vec![(1., "1".to_owned()), (2., "2".to_owned())];
        // Unchanged fields have nothing to apply.
        assert!(!has_unsent_edits(
            &["1".to_owned(), "2".to_owned()],
            &initial,
            None
        ));
        // A typed field keeps Apply enabled.
        assert!(has_unsent_edits(
            &["1".to_owned(), "3".to_owned()],
            &initial,
            None
        ));
        // Fields whose texts were just submitted disable it until the
        // project answers and the form refreshes: no second send.
        let submitted = vec!["1".to_owned(), "3".to_owned()];
        assert!(!has_unsent_edits(&submitted, &initial, Some(&submitted)));
        // A later change past the submitted texts re-enables it.
        assert!(has_unsent_edits(
            &["1".to_owned(), "4".to_owned()],
            &initial,
            Some(&submitted)
        ));
    }
}
