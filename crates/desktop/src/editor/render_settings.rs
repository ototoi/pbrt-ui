//! Draft render settings; Enter and blur commit a revision-checked DTO to core.
use super::Session;
use crate::backend::Command;
use gpui_kit::{
    component::{
        Sizable,
        input::{InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
    },
    *,
};
use protocol::{Revision, jobs::RenderSettings};
use std::time::Duration;

pub(super) fn flipped_gpu_settings(
    state: &serde_json::Value,
) -> Option<(RenderSettings, Revision)> {
    let mut settings = serde_json::from_value::<RenderSettings>(state["settings"].clone()).ok()?;
    let revision = serde_json::from_value::<Revision>(state["settings_revision"].clone()).ok()?;
    settings.gpu = !settings.gpu;
    Some((settings, revision))
}

pub struct SettingsForm {
    session: Entity<Session>,
    inputs: [Entity<InputState>; 2],
    baseline: [String; 2],
    revision: Revision,
    pending: Option<[String; 2]>,
    gpu: bool,
    message: String,
    step_generation: u64,
}
/// Threads is an execution-environment setting. Samples overrides the scene's
/// Sampler for renders started here only; blank leaves image quality to the scene.
fn fields(settings: &RenderSettings) -> [String; 2] {
    [
        settings.threads.map(|v| v.to_string()).unwrap_or_default(),
        settings
            .samples_per_pixel
            .map(|v| v.to_string())
            .unwrap_or_default(),
    ]
}
const BLANK: [&str; 2] = ["auto", "scene"];
fn parse(values: &[String; 2], gpu: bool) -> Result<RenderSettings, String> {
    let number = |i: usize, label: &str| {
        values[i]
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("{label} must be a positive integer"))
    };
    let optional = |i: usize, label: &str| {
        (!values[i].trim().is_empty())
            .then(|| number(i, label))
            .transpose()
    };
    let threads = optional(0, "Threads")?;
    let samples_per_pixel = optional(1, "Samples")?;
    if threads.is_some_and(|n| n == 0 || n > 1024)
        || samples_per_pixel.is_some_and(|n| n == 0 || n > 1_048_576)
    {
        return Err("Settings exceed the supported limits".into());
    }
    Ok(RenderSettings {
        samples_per_pixel,
        threads,
        gpu,
        max_depth: None,
    })
}

/// Why one field's text cannot be saved, checked on its own so an invalid
/// draft in one field never holds back the other.
fn field_error(index: usize, text: &str) -> Option<String> {
    let mut values = [String::new(), String::new()];
    values[index] = text.to_owned();
    parse(&values, false).err()
}

/// One spin box step. Blank (auto / scene) steps up to the step size; a
/// step down from 1 returns to blank, so the field can go back to its default.
fn stepped_setting(text: &str, action: StepAction, size: u32) -> String {
    match (text.trim().parse::<u32>().ok().filter(|&n| n > 0), action) {
        (None, StepAction::Increment) => size.to_string(),
        (None, StepAction::Decrement) => String::new(),
        (Some(n), StepAction::Increment) => n.saturating_add(size).to_string(),
        (Some(1), StepAction::Decrement) => String::new(),
        (Some(n), StepAction::Decrement) => n.saturating_sub(size).max(1).to_string(),
    }
}

/// Enter and blur commit an edited field.
#[derive(Debug)]
enum SettingsCommit {
    /// The fields still hold their baseline text, or the settings already
    /// on their way: nothing to send.
    Unchanged,
    /// The valid changed fields to save, and the error of an invalid one
    /// that keeps its baseline value meanwhile.
    Apply(RenderSettings, Option<String>),
    /// Only invalid changes: the error message shows.
    Invalid(String),
}

/// The commit decision Enter and blur share. Enter then blur before the
/// backend answers must not resend: the second write would carry the old
/// revision and report a conflict.
fn settings_commit(
    values: &[String; 2],
    baseline: &[String; 2],
    pending: Option<&[String; 2]>,
    gpu: bool,
) -> SettingsCommit {
    let errors: [Option<String>; 2] = std::array::from_fn(|i| field_error(i, &values[i]));
    let error = errors.iter().flatten().next().cloned();
    let merged: [String; 2] = std::array::from_fn(|i| {
        if errors[i].is_some() {
            baseline[i].clone()
        } else {
            values[i].clone()
        }
    });
    let settings = (merged != *baseline)
        .then(|| parse(&merged, gpu).ok())
        .flatten()
        .filter(|settings| pending != Some(&fields(settings)));
    match (settings, error) {
        (Some(settings), error) => SettingsCommit::Apply(settings, error),
        (None, Some(error)) => SettingsCommit::Invalid(error),
        (None, None) => SettingsCommit::Unchanged,
    }
}
impl SettingsForm {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings =
            serde_json::from_value::<RenderSettings>(session.read(cx).settings["settings"].clone())
                .unwrap_or_default();
        let baseline = fields(&settings);
        let gpu = settings.gpu;
        let inputs = std::array::from_fn(|i| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(BLANK[i])
                    .default_value(baseline[i].clone())
            })
        });
        for input in &inputs {
            input.update(cx, |state, cx| state.set_step(None, window, cx));
            cx.observe(input, |_, _, cx| cx.notify()).detach();
            // Blur commits a changed, valid edit too, so tabbing away from a
            // fresh value still saves it.
            cx.subscribe(input, |this, _, event: &InputEvent, cx| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit(cx),
                _ => {}
            })
            .detach();
            cx.subscribe_in(
                input,
                window,
                |this, input, event: &NumberInputEvent, window, cx| {
                    let NumberInputEvent::Step(action) = event;
                    let size = super::parameter_field::step_size(window) as u32;
                    this.step(input, *action, size, window, cx);
                },
            )
            .detach();
        }
        cx.observe_in(&session, window, |this, _, window, cx| {
            this.sync(window, cx)
        })
        .detach();
        Self {
            session,
            inputs,
            baseline,
            revision: Revision::initial(),
            pending: None,
            gpu,
            message: String::new(),
            step_generation: 0,
        }
    }
    fn values(&self, cx: &App) -> [String; 2] {
        std::array::from_fn(|i| self.inputs[i].read(cx).value().to_string())
    }
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.session.read(cx).settings;
        let Ok(settings) = serde_json::from_value::<RenderSettings>(state["settings"].clone())
        else {
            return;
        };
        let Ok(revision) = serde_json::from_value::<Revision>(state["settings_revision"].clone())
        else {
            return;
        };
        let incoming = fields(&settings);
        self.gpu = settings.gpu;
        // With no Reload control, a change from elsewhere (CLI, agent)
        // replaces what is typed here instead of blocking it.
        let own = self.pending.as_ref() == Some(&incoming);
        if self.revision != revision || self.values(cx) == self.baseline || own {
            self.baseline = incoming;
            self.revision = revision;
            self.pending = None;
            let values = self.values(cx);
            let mut kept = false;
            for (i, value) in values.iter().enumerate() {
                // Our own write saved the other field: an invalid draft here stays.
                if own && field_error(i, value).is_some() {
                    kept = true;
                    continue;
                }
                self.inputs[i].update(cx, |input, cx| {
                    input.set_value(self.baseline[i].clone(), window, cx)
                });
            }
            if !kept {
                self.message.clear();
            }
        }
        cx.notify();
    }
    fn commit(&mut self, cx: &mut Context<Self>) {
        let values = self.values(cx);
        match settings_commit(&values, &self.baseline, self.pending.as_ref(), self.gpu) {
            SettingsCommit::Unchanged => {}
            SettingsCommit::Apply(settings, error) => {
                self.message = error.unwrap_or_default();
                self.pending = Some(fields(&settings));
                let expected = self.revision.clone();
                self.session.update(cx, |s, cx| {
                    s.send(Command::Settings(settings, expected), cx)
                });
            }
            SettingsCommit::Invalid(message) => self.message = message,
        }
        cx.notify();
    }
}
impl SettingsForm {
    /// Undo/Redo keys belong to the project unless a field holds an edit of
    /// its own: focus stays in a spin box after a step, and its text undo
    /// would otherwise swallow Ctrl+Z (as the Inspector fields already do).
    fn forwards_history_keys(&self, cx: &App) -> bool {
        self.pending.is_none() && self.values(cx) == self.baseline
    }

    /// Step one field and commit once the steps pause, so repeated presses
    /// make one settings write.
    fn step(
        &mut self,
        input: &Entity<InputState>,
        action: StepAction,
        size: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = stepped_setting(&input.read(cx).value(), action, size);
        input.update(cx, |state, cx| state.set_value(value, window, cx));
        self.step_generation = self.step_generation.wrapping_add(1);
        let generation = self.step_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.step_generation == generation {
                    this.commit(cx);
                }
            });
        })
        .detach();
    }
}

impl Render for SettingsForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Undo, window, cx| {
                    if this.forwards_history_keys(cx) {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(super::menu::UndoEdit), cx);
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Redo, window, cx| {
                    if this.forwards_history_keys(cx) {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(super::menu::RedoEdit), cx);
                    }
                },
            ))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_2()
            .pb_2()
            .text_xs();
        for (i, label) in ["Threads", "Samples"].into_iter().enumerate() {
            row = row.child(
                div()
                    .w(px(130.))
                    .on_action(
                        cx.listener(move |this, _: &super::menu::StepUpTen, window, cx| {
                            let input = this.inputs[i].clone();
                            this.step(&input, StepAction::Increment, 10, window, cx);
                        }),
                    )
                    .on_action(cx.listener(
                        move |this, _: &super::menu::StepDownTen, window, cx| {
                            let input = this.inputs[i].clone();
                            this.step(&input, StepAction::Decrement, 10, window, cx);
                        },
                    ))
                    .child(label)
                    .child(NumberInput::new(&self.inputs[i]).small()),
            );
        }
        row.child(self.message.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::{
        SettingsCommit, StepAction, flipped_gpu_settings, parse, settings_commit, stepped_setting,
    };
    #[test]
    fn steps_return_to_blank_below_one_and_shift_steps_by_ten() {
        assert_eq!(stepped_setting("", StepAction::Increment, 1), "1");
        assert_eq!(stepped_setting("", StepAction::Increment, 10), "10");
        assert_eq!(stepped_setting("1", StepAction::Decrement, 1), "");
        assert_eq!(stepped_setting("1", StepAction::Decrement, 10), "");
        assert_eq!(stepped_setting("", StepAction::Decrement, 1), "");
        assert_eq!(stepped_setting("4", StepAction::Decrement, 1), "3");
        assert_eq!(stepped_setting("4", StepAction::Decrement, 10), "1");
        assert_eq!(stepped_setting("4", StepAction::Increment, 10), "14");
    }
    #[test]
    fn an_invalid_field_does_not_hold_back_the_other() {
        let SettingsCommit::Apply(settings, Some(error)) = settings_commit(
            &["8".into(), ".".into()],
            &["".into(), "".into()],
            None,
            false,
        ) else {
            panic!("the valid Threads saves while Samples reports its error");
        };
        assert_eq!(
            (settings.threads, settings.samples_per_pixel),
            (Some(8), None)
        );
        assert!(error.contains("Samples"), "{error}");
    }
    #[test]
    fn blank_fields_use_scene_and_renderer_defaults() {
        let settings = parse(&[" ".into(), " ".into()], false).unwrap();
        assert_eq!((settings.samples_per_pixel, settings.threads), (None, None));
        assert_eq!(super::fields(&settings), ["", ""]);
    }
    #[test]
    fn fields_validate_and_parse_limits() {
        let settings = parse(&["4".into(), "32".into()], false).unwrap();
        assert_eq!(
            (settings.threads, settings.samples_per_pixel),
            (Some(4), Some(32))
        );
        for input in [
            ["1.5".into(), "".into()],
            ["0".into(), "".into()],
            ["1025".into(), "".into()],
            ["".into(), "0".into()],
            ["".into(), "1048577".into()],
        ] {
            assert!(parse(&input, false).is_err());
        }
        assert!(parse(&["4".into(), "".into()], true).unwrap().gpu);
    }
    #[test]
    fn gpu_toggle_preserves_other_settings() {
        let (settings, revision) = flipped_gpu_settings(&serde_json::json!({
            "settings": {"threads": 6, "samples_per_pixel": 32, "gpu": false},
            "settings_revision": "7"
        }))
        .unwrap();
        assert!(settings.gpu);
        assert_eq!(
            (settings.threads, settings.samples_per_pixel),
            (Some(6), Some(32))
        );
        assert_eq!(revision.0, "7");
    }
    #[test]
    fn enter_and_blur_share_commit_behavior() {
        assert!(matches!(
            settings_commit(
                &["4".into(), "32".into()],
                &["".into(), "".into()],
                None,
                true
            ),
            SettingsCommit::Apply(_, None)
        ));
        assert!(matches!(
            settings_commit(
                &[" ".into(), "".into()],
                &["4".into(), "32".into()],
                None,
                true
            ),
            SettingsCommit::Apply(_, None)
        ));
        assert!(matches!(
            settings_commit(
                &["4".into(), "32".into()],
                &["4".into(), "32".into()],
                None,
                true
            ),
            SettingsCommit::Unchanged
        ));
        assert!(matches!(
            settings_commit(
                &["1.5".into(), "".into()],
                &["".into(), "".into()],
                None,
                true
            ),
            SettingsCommit::Invalid(_)
        ));
        assert!(matches!(
            settings_commit(
                &["4".into(), "32".into()],
                &["".into(), "".into()],
                Some(&["4".into(), "32".into()]),
                true
            ),
            SettingsCommit::Unchanged
        ));
    }
}
