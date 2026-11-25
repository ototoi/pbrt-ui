//! Small typed parameter editor; large arrays and resource bindings stay read-only.
use super::Session;
use crate::backend::{Command, Follow};
use gpui_kit::{
    component::{
        Sizable,
        button::Button,
        input::{Input, InputState},
    },
    *,
};
use protocol::project::ParameterValues;

pub struct ParameterForm {
    session: Entity<Session>,
    target: serde_json::Value,
    project: serde_json::Value,
    name: String,
    value_type: String,
    color_space: Option<String>,
    kind: String,
    value_count: usize,
    input: Entity<InputState>,
    /// The value shown when the form was built, to tell unsent typing.
    initial: String,
    /// Stable across revisions: `parameter:<target id>:<name>`.
    form: String,
    message: String,
}

impl ParameterForm {
    pub fn supports(parameter: &serde_json::Value) -> bool {
        !parameter["preview_truncated"].as_bool().unwrap_or(true)
            && matches!(
                parameter["values"]["kind"].as_str(),
                Some("floats" | "integers" | "booleans" | "strings" | "assets" | "builtin_spectra")
            )
    }
    pub fn new(
        session: Entity<Session>,
        target: serde_json::Value,
        project: serde_json::Value,
        parameter: &serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input_value = serde_json::json!({"kind":parameter["values"]["kind"],"value":parameter["values"]["value"]}).to_string();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(input_value.clone()));
        let form = format!(
            "parameter:{}:{}",
            target["id"].as_str().unwrap_or_default(),
            parameter["name"].as_str().unwrap_or_default()
        );
        cx.observe(&input, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            target,
            project,
            name: parameter["name"].as_str().unwrap_or_default().to_owned(),
            value_type: parameter["value_type"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            color_space: parameter_color_space(parameter),
            kind: parameter["values"]["kind"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            value_count: parameter["value_count"]
                .as_str()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            input,
            initial: input_value,
            form,
            message: String::new(),
        }
    }

    /// The user typed something that was not sent.
    pub fn is_modified(&self, cx: &App) -> bool {
        self.input.read(cx).value() != self.initial.as_str()
    }
    fn apply(&mut self, cx: &mut Context<Self>) {
        let raw = self.input.read(cx).value().to_string();
        let result = serde_json::from_str::<ParameterValues>(&raw)
            .map_err(|e| format!("Invalid typed values: {e}"));
        let values = match result {
            Ok(values) => values,
            Err(error) => {
                self.message = error;
                cx.notify();
                return;
            }
        };
        let count = match &values {
            ParameterValues::Floats(v) => v.len(),
            ParameterValues::Integers(v) => v.len(),
            ParameterValues::Booleans(v) => v.len(),
            ParameterValues::Strings(v) => v.len(),
            ParameterValues::Assets(v) => v.len(),
            ParameterValues::BuiltinSpectra(v) => v.len(),
            ParameterValues::Resources { ids, .. } => ids.len(),
        };
        if values_kind(&values) != self.kind || count != self.value_count {
            self.message = "Keep the parameter's value type and number of values unchanged".into();
            cx.notify();
            return;
        }
        let project = self.session.read(cx).project_value();
        if project != self.project || !project.is_object() {
            self.message =
                "Project changed; wait for the Inspector to refresh before applying".into();
            cx.notify();
            return;
        }
        let edit = serde_json::json!({"kind":"set_parameter","target":self.target,"name":self.name,"value_type":self.value_type,"value":{"name":self.name,"value_type":self.value_type,"color_space":self.color_space,"values":values}});
        let form = self.form.clone();
        self.session.update(cx, |session, cx| {
            session.edit_feedback.submitted(&form);
            session.send(
                Command::Apply {
                    project: self.project.clone(),
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
/// The form's own validation message, else the core's answer for it.
pub fn feedback_text(local: &str, session: &Entity<Session>, form: &str, cx: &App) -> String {
    if !local.is_empty() {
        return local.to_owned();
    }
    session
        .read(cx)
        .edit_feedback
        .message(form)
        .unwrap_or_default()
        .to_owned()
}
fn parameter_color_space(parameter: &serde_json::Value) -> Option<String> {
    parameter["color_space"]["value"]
        .as_str()
        .map(str::to_owned)
}
fn values_kind(values: &ParameterValues) -> &'static str {
    match values {
        ParameterValues::Floats(_) => "floats",
        ParameterValues::Integers(_) => "integers",
        ParameterValues::Booleans(_) => "booleans",
        ParameterValues::Strings(_) => "strings",
        ParameterValues::Assets(_) => "assets",
        ParameterValues::BuiltinSpectra(_) => "builtin_spectra",
        ParameterValues::Resources { .. } => "resources",
    }
}
impl Render for ParameterForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(rgb(0x34373e))
            .child(div().text_xs().text_color(rgb(0x9098a6)).child(format!(
                "{} · {} · {} values",
                self.name, self.value_type, self.value_count
            )))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x858b97))
                    .child("Edit JSON with the same kind and number of values"),
            )
            .child(Input::new(&self.input).small().w_full())
            .child(
                Button::new(format!("apply-parameter-{}:{}", self.value_type, self.name))
                    .icon(gpui_kit::assets::IconName::Check)
                    .label("Apply Values")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0xb8bbc4))
                    .child(feedback_text(&self.message, &self.session, &self.form, cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{ParameterForm, parameter_color_space};
    use serde_json::json;
    #[test]
    fn editing_preserves_explicit_color_space_and_does_not_make_truncated_values_editable() {
        let mut p = json!({"values":{"kind":"strings","value":["prefix"]},"preview_truncated":true,"color_space":{"kind":"explicit","value":"aces2065-1"}});
        assert_eq!(parameter_color_space(&p).as_deref(), Some("aces2065-1"));
        assert!(!ParameterForm::supports(&p));
        p["preview_truncated"] = json!(false);
        p["color_space"] = json!({"kind":"inherit"});
        assert!(ParameterForm::supports(&p));
        assert_eq!(parameter_color_space(&p), None);
    }
}
