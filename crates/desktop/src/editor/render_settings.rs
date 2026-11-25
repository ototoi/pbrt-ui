//! Draft render settings; Enter and blur commit a revision-checked DTO to core.
use super::Session;
use crate::backend::Command;
use gpui_kit::{
    component::{
        Sizable,
        input::{Input, InputEvent, InputState},
    },
    *,
};
use protocol::{Revision, jobs::RenderSettings};

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
    inputs: [Entity<InputState>; 1],
    baseline: [String; 1],
    revision: Revision,
    pending: Option<[String; 1]>,
    gpu: bool,
    message: String,
}
/// Threads is an execution-environment setting; image quality comes from the scene.
fn fields(settings: &RenderSettings) -> [String; 1] {
    [settings.threads.map(|v| v.to_string()).unwrap_or_default()]
}
const BLANK: [&str; 1] = ["auto"];
fn parse(values: &[String; 1], gpu: bool) -> Result<RenderSettings, String> {
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
    if threads.is_some_and(|n| n == 0 || n > 1024) {
        return Err("Settings exceed the supported limits".into());
    }
    Ok(RenderSettings {
        samples_per_pixel: None,
        threads,
        gpu,
        max_depth: None,
    })
}

/// Enter and blur commit an edited Threads field.
enum ThreadsCommit {
    /// The field still holds its baseline text, or the settings already
    /// on their way: nothing to send.
    Unchanged,
    /// A changed, valid field: the settings to save.
    Apply(RenderSettings),
    /// A changed, invalid field: today's Apply error message still shows.
    Invalid(String),
}

/// The commit decision Enter and blur share: an unchanged field sends
/// nothing, a changed valid field applies, a changed invalid one keeps
/// the error feedback the removed Apply button showed.
/// Enter then blur before the backend answers must not resend: the
/// second write would carry the old revision and report a conflict.
fn threads_commit(
    values: &[String; 1],
    baseline: &[String; 1],
    pending: Option<&[String; 1]>,
    gpu: bool,
) -> ThreadsCommit {
    if values == baseline {
        ThreadsCommit::Unchanged
    } else {
        match parse(values, gpu) {
            Ok(settings) if pending == Some(&fields(&settings)) => ThreadsCommit::Unchanged,
            Ok(settings) => ThreadsCommit::Apply(settings),
            Err(message) => ThreadsCommit::Invalid(message),
        }
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
            cx.observe(input, |_, _, cx| cx.notify()).detach();
            // Enter commits the edited value.
            // did; blur commits an edit only when it changed and is
            // valid, so tabbing away from a fresh value still saves it.
            cx.subscribe(input, |this, _, event: &InputEvent, cx| match event {
                InputEvent::PressEnter { .. } => this.commit(cx),
                InputEvent::Blur => this.commit(cx),
                _ => {}
            })
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
        }
    }
    fn values(&self, cx: &App) -> [String; 1] {
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
        // With no Reload control, a change from elsewhere (CLI,
        // agent) replaces what is typed here instead of blocking it.
        if self.revision != revision
            || self.values(cx) == self.baseline
            || self.pending.as_ref() == Some(&incoming)
        {
            self.baseline = incoming;
            self.revision = revision;
            self.pending = None;
            self.message.clear();
            for i in 0..self.inputs.len() {
                self.inputs[i].update(cx, |input, cx| {
                    input.set_value(self.baseline[i].clone(), window, cx)
                });
            }
        }
        cx.notify();
    }
    /// Enter and blur land here: an unchanged field sends nothing,
    /// a changed valid one saves, a changed invalid one keeps the error.
    fn commit(&mut self, cx: &mut Context<Self>) {
        let values = self.values(cx);
        match threads_commit(&values, &self.baseline, self.pending.as_ref(), self.gpu) {
            ThreadsCommit::Unchanged => {}
            ThreadsCommit::Apply(settings) => {
                self.pending = Some(fields(&settings));
                let expected = self.revision.clone();
                self.session.update(cx, |s, cx| {
                    s.send(Command::Settings(settings, expected), cx)
                });
            }
            ThreadsCommit::Invalid(message) => self.message = message,
        }
        cx.notify();
    }
}
impl Render for SettingsForm {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_2()
            .pb_2()
            .text_xs();
        for (i, label) in ["Threads"].into_iter().enumerate() {
            row = row.child(
                div()
                    .w(px(90.))
                    .child(label)
                    .child(Input::new(&self.inputs[i]).small()),
            );
        }
        // Enter applies, blur applies a changed valid edit; blank keeps
        // meaning all cores, and parse errors show here.
        row.child(self.message.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::{ThreadsCommit, flipped_gpu_settings, parse, threads_commit};
    /// Scene quality comes from its Sampler and Integrator; blank Threads uses all cores.
    #[test]
    fn blank_samples_and_threads_leave_them_to_the_scene_and_renderer() {
        let settings = parse(&[" ".into()], false).unwrap();
        assert_eq!(
            (
                settings.samples_per_pixel,
                settings.threads,
                settings.max_depth
            ),
            (None, None, None)
        );
        assert_eq!(super::fields(&settings), [""]);
    }

    #[test]
    fn thread_integer_limits_are_validated_before_commit() {
        let settings = parse(&["4".into()], false).unwrap();
        assert_eq!(
            (
                settings.samples_per_pixel,
                settings.threads,
                settings.max_depth
            ),
            (None, Some(4), None)
        );
        assert!(parse(&["1.5".into()], false).is_err());
        assert!(parse(&["0".into()], false).is_err());
        assert!(parse(&["1025".into()], false).is_err());
        assert!(parse(&["4".into()], true).unwrap().gpu);
    }

    #[test]
    fn gpu_toggle_preserves_threads_and_advances_only_the_gpu_flag() {
        let (settings, revision) = flipped_gpu_settings(&serde_json::json!({
            "settings": {"threads": 6, "gpu": false}, "settings_revision": "7"
        }))
        .unwrap();
        assert!(settings.gpu);
        assert_eq!(settings.threads, Some(6));
        assert_eq!(revision.0, "7");
    }

    /// Enter (and blur after an edit) is the only way to commit the
    /// thread limit now the Apply button is gone.
    #[test]
    fn enter_applies_the_thread_count_and_blank_means_all_cores() {
        // A changed valid field applies: Enter and blur share this commit.
        let ThreadsCommit::Apply(settings) =
            threads_commit(&["4".into()], &["".into()], None, true)
        else {
            panic!("a changed valid field should apply");
        };
        assert_eq!(settings.threads, Some(4));
        assert!(settings.gpu);
        // An emptied field applies all cores, as blank always did.
        let ThreadsCommit::Apply(settings) =
            threads_commit(&[" ".into()], &["4".into()], None, true)
        else {
            panic!("clearing the field should apply the default");
        };
        assert_eq!(settings.threads, None);
        // An unchanged field sends nothing: blurring without edits must
        // not stack a no-op revision.
        assert!(matches!(
            threads_commit(&["4".to_owned()], &["4".to_owned()], None, true),
            ThreadsCommit::Unchanged
        ));
        // An invalid edit keeps today's Apply error message.
        assert!(matches!(
            threads_commit(&["1.5".into()], &["".into()], None, true),
            ThreadsCommit::Invalid(_)
        ));
        // Enter then blur before the save answers: the blur sends nothing.
        assert!(matches!(
            threads_commit(&["4".into()], &["".into()], Some(&["4".into()]), true),
            ThreadsCommit::Unchanged
        ));
    }
}
