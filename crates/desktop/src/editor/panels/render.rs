//! Render controls and artifact display.
use super::super::EditorPanel;
use crate::backend::Command;
use crate::editor::jobs::{JobRow, elapsed_label, job_rows, state_color, track_elapsed};
use crate::editor::menu::OpenRenderSettings;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    spinner::Spinner,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;
use std::time::Instant;
impl EditorPanel {
    pub(super) fn render_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let has_scene = session.project.active_scene.is_some();
        let missing_camera =
            missing_render_camera(has_scene, session.render_camera_label.as_deref());
        let render_starting = session.render_start.active();
        let ipr_enabled = session.ipr["enabled"].as_bool().unwrap_or(false);
        let rendering = render_running(&session.jobs);
        let render_artifact = session.render_artifact;
        let mut controls = div().flex().flex_wrap().items_center().gap_2().p_2();
        for (id, icon, label, tooltip, current_view) in [
            (
                "render-scene",
                assets::IconName::Play,
                "Render Scene",
                "Render through the scene camera (Ctrl+Enter)",
                false,
            ),
            (
                "render-view",
                assets::IconName::Camera,
                "Render View",
                "Render the Scene panel's current view (Ctrl+Shift+Enter)",
                true,
            ),
        ] {
            let session = self.session.clone();
            controls = controls.child(
                Button::new(id)
                    .icon(icon)
                    .label(if render_starting {
                        "Starting…"
                    } else {
                        label
                    })
                    .small()
                    .disabled(!has_scene || render_starting)
                    .tooltip(tooltip)
                    .on_click(move |_, _, cx| {
                        session.update(cx, |s, cx| s.start_render(current_view, cx));
                    }),
            );
        }
        // The buttons appear in this order: Render Scene / Render View / Cancel /
        // Render Settings / Export, with the camera note after them.
        let ipr_label = ipr_status_label(ipr_enabled, &session.ipr);
        let gpu_enabled = session.settings["settings"]["gpu"] == true;
        let gpu_session = self.session.clone();
        controls = controls.child(
            Button::new("gpu-toggle")
                .label("GPU")
                .small()
                .when(gpu_enabled, |button| button.primary())
                .toggled(gpu_enabled)
                .disabled(!has_scene && !gpu_enabled)
                .tooltip("Use the GPU for final renders and Render View")
                .on_click(move |_, _, cx| {
                    let state = gpu_session.read(cx).settings.clone();
                    if let Some((settings, revision)) =
                        super::super::render_settings::flipped_gpu_settings(&state)
                    {
                        gpu_session.update(cx, |session, cx| {
                            session.send(Command::Settings(settings, revision), cx)
                        });
                    }
                }),
        );
        {
            let session = self.session.clone();
            controls = controls.child(
                Button::new("cancel-render")
                    .icon(assets::IconName::Square)
                    .label("Cancel")
                    .small()
                    .disabled(!rendering)
                    .tooltip("Cancel the running render (Ctrl+.)")
                    .on_click(move |_, _, cx| {
                        session.update(cx, |s, cx| s.send(Command::CancelRender, cx));
                    }),
            );
        }
        controls = controls.child(
            Button::new("render-settings")
                .icon(assets::IconName::Settings)
                .label("Render Settings")
                .small()
                .disabled(!has_scene)
                .tooltip("Choose the Camera used by Render Scene")
                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenRenderSettings), cx)),
        );
        // Export still acts on the shown image and appears only once
        // one exists, but now sits in the controls row.
        if let Some(id) = render_artifact {
            let session = self.session.clone();
            controls = controls.child(
                Button::new("export-result")
                    .icon(assets::IconName::Download)
                    .label("Export")
                    .small()
                    .tooltip("Save the result as an image")
                    .accessibility_label("Export result")
                    .on_click(move |_, _, cx| {
                        let directory = std::env::var_os("HOME")
                            .map(PathBuf::from)
                            .unwrap_or_else(std::env::temp_dir);
                        let picker = cx.prompt_for_new_path(&directory, Some("pbrt-result.png"));
                        let session = session.clone();
                        cx.spawn(async move |cx| {
                            if let Ok(Ok(Some(path))) = picker.await {
                                session.update(cx, |s, cx| s.send(Command::Export(id, path), cx));
                            }
                        })
                        .detach();
                    }),
            );
        }
        let camera_label = session.render_camera_label.as_deref().unwrap_or("None");
        controls = controls.child(
            div()
                .px_1()
                .text_xs()
                .text_color(rgb(0x9aa1ad))
                .child(format!("Camera: {camera_label}")),
        );
        let image = session.render_image.clone();
        let rows = job_rows(&session.jobs);
        // The revision of the result shown above (null before the first
        // render); the stale note marks edits that postdate it.
        let revision = session.render_project.is_object().then(|| {
            (
                session.render_project["revision"]
                    .as_str()
                    .unwrap_or("?")
                    .to_owned(),
                session.render_project != session.project_value(),
            )
        });
        let revision = revision
            .as_ref()
            .map(|(revision, stale)| (revision.as_str(), *stale));
        let render_job = latest_render_job(&rows);
        let now = Instant::now();
        track_elapsed(&rows, &mut self.job_times, now);
        let elapsed = render_job
            .and_then(|job| self.job_times.get(&job.job_id))
            .map(|(start, done)| elapsed_label(done.unwrap_or_else(|| now - *start)))
            .unwrap_or_default();
        // Render information only: the other jobs stay in the Jobs panel
        // and the Console. Each status line reads the fraction of its own
        // kind's job, gated by that job still running.
        let render_received = running_render_fraction(session.render_received.as_ref(), render_job);
        let ipr_received = active_ipr_fraction(
            session.render_received.as_ref(),
            session.ipr["active_job_id"].as_str(),
        );
        let mut status_lines = render_status_lines(
            render_job,
            &elapsed,
            ipr_label,
            revision,
            render_received,
            ipr_received,
        );
        if let Some(error) = &session.render_request_error {
            status_lines.push(RenderStatusLine {
                text: format!("Render not started · {error}"),
                color: state_color("failed"),
                spinner: false,
            });
        }
        if session.render_project == session.project_value()
            && let Some(warning) = medium_dark_warning(
                session.render_mean_luminance,
                session.project_tree["camera_medium_optical_depth"].as_f64(),
            )
        {
            status_lines.push(RenderStatusLine {
                text: warning,
                color: rgb(0xe6b65c),
                spinner: false,
            });
        }
        let status = div()
            .p_2()
            .text_xs()
            .children(status_lines.into_iter().map(|line| {
                div()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    // Non-blocking status: a small spinner beside the live
                    // lines.
                    .when(line.spinner, |row| {
                        row.child(Spinner::new().color(line.color.into()).xsmall())
                    })
                    .text_color(line.color)
                    // A long reason wraps instead of running past the panel.
                    .child(div().flex_1().min_w_0().child(line.text))
            }));
        let display = div()
            .flex_1()
            .min_h_0()
            .relative()
            .w_full()
            .overflow_hidden()
            .bg(rgb(0x17191e))
            .when_some(image, |display, image| {
                display.child(
                    img(image)
                        .absolute()
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                )
            });
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(controls)
            .when(missing_camera, |panel| {
                panel.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .pb_2()
                        .text_xs()
                        .text_color(rgb(0xe6b65c))
                        .child("No Camera is selected for Render Scene.")
                        .child(
                            Button::new("open-render-settings-missing-camera")
                                .label("Choose in Render Settings")
                                .small()
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(OpenRenderSettings), cx)
                                }),
                        ),
                )
            })
            .children(self.settings_form.clone())
            .child(display)
            .child(status)
            .into_any_element()
    }
}

fn missing_render_camera(has_scene: bool, camera_label: Option<&str>) -> bool {
    has_scene && camera_label.is_none()
}

/// The newest `render` job (Render Scene and Render View share the kind) —
/// the only job the Render panel's status area describes. Other kinds stay
/// in the Jobs panel and the Console.
fn latest_render_job(rows: &[JobRow]) -> Option<&JobRow> {
    rows.iter().find(|row| row.kind == "render")
}

/// The live IPR state as a footer label; `None` while IPR is off.
fn ipr_status_label(enabled: bool, ipr: &serde_json::Value) -> Option<&'static str> {
    if !enabled {
        return None;
    }
    if ipr["active_job_id"].is_string() {
        Some("rendering…")
    } else if ipr["waiting_for_final_render"].as_bool().unwrap_or(false) {
        Some("waiting for the final render")
    } else {
        Some("watching for edits")
    }
}

/// One line of the Render panel's status area. `spinner` marks the live
/// states (a running final render, an IPR that is rendering), drawn with the
/// line's colour.
pub(super) struct RenderStatusLine {
    pub text: String,
    pub color: Rgba,
    pub spinner: bool,
}

/// The Render panel's status lines: the shown result's revision (with its
/// stale note), the newest render job's state, and the IPR state. Pure so
/// the layout can be tested without a window. `render_received` /
/// `ipr_received` are the pixel fractions (0..=1) of the running final
/// render's / the running IPR job's partial image, if known.
pub(super) fn render_status_lines(
    render_job: Option<&JobRow>,
    elapsed: &str,
    ipr_label: Option<&str>,
    revision: Option<(&str, bool)>,
    render_received: Option<f64>,
    ipr_received: Option<f64>,
) -> Vec<RenderStatusLine> {
    let mut lines = Vec::new();
    if let Some((revision, stale)) = revision {
        lines.push(RenderStatusLine {
            text: format!(
                "Project revision {revision}{}",
                if stale {
                    " · stale: project has changed"
                } else {
                    ""
                }
            ),
            color: rgb(0x9aa1ad),
            spinner: false,
        });
    }
    if let Some(job) = render_job {
        let active = job.active;
        let mut text = format!("Render · {}", job.state);
        if active {
            if !elapsed.is_empty() {
                text.push_str(&format!(" ({elapsed})"));
            }
            if !job.phase.is_empty() {
                text.push_str(&format!(" · {}", job.phase));
            }
            if let Some(fraction) = render_received {
                text.push_str(&format!(" · {}", pixels_done_text(fraction)));
            }
        } else {
            match job.state.as_str() {
                "succeeded" if !elapsed.is_empty() => {
                    text.push_str(&format!(" ({elapsed})"));
                }
                // Failed jobs carry their error text as their detail.
                "failed" if !job.detail.is_empty() => {
                    text.push_str(&format!(" · {}", job.detail));
                }
                _ => {}
            }
        }
        lines.push(RenderStatusLine {
            text,
            color: state_color(&job.state),
            spinner: active,
        });
    }
    if let Some(label) = ipr_label {
        let mut text = label.to_owned();
        if label == "rendering…"
            && let Some(fraction) = ipr_received
        {
            text.push_str(&format!(" · {}", pixels_done_text(fraction)));
        }
        lines.push(RenderStatusLine {
            text,
            color: rgb(0x8d93a1),
            spinner: label == "rendering…",
        });
    }
    lines
}

/// "12% of pixels" — how far a running render has come, rounded down to
/// a whole percent of the image area.
fn medium_dark_warning(luminance: Option<f64>, optical_depth: Option<f64>) -> Option<String> {
    let luminance = luminance?;
    let optical_depth = optical_depth?;
    (luminance <= 0.015 && optical_depth >= 8.)
        .then(|| format!("Camera medium may absorb everything (σ·distance ≈ {optical_depth:.1})"))
}

fn pixels_done_text(fraction: f64) -> String {
    format!("{}% of pixels", (fraction.clamp(0., 1.) * 100.) as u64)
}

/// The pixel fraction for the final-render status line: shown only while
/// the shown partial belongs to that very job and the job still runs, so
/// a finished render — or a newer render queued over another's image —
/// inherits nothing.
fn running_render_fraction(received: Option<&(String, f64)>, job: Option<&JobRow>) -> Option<f64> {
    let (job_id, fraction) = received?;
    job.is_some_and(|job| job.active && job.job_id == *job_id)
        .then_some(*fraction)
}

/// The pixel fraction for the IPR "rendering…" line: shown only while the
/// shown frames belong to the currently active IPR job.
fn active_ipr_fraction(
    received: Option<&(String, f64)>,
    ipr_active_job_id: Option<&str>,
) -> Option<f64> {
    let (job_id, fraction) = received?;
    (Some(job_id.as_str()) == ipr_active_job_id).then_some(*fraction)
}

/// Whether a final render is queued or running: what Cancel acts on. IPR
/// previews are stopped with the IPR toggle instead.
fn render_running(jobs: &serde_json::Value) -> bool {
    jobs["jobs"].as_array().into_iter().flatten().any(|job| {
        job["kind"] == "render"
            && matches!(
                job["state"].as_str(),
                Some("queued" | "running" | "cancelling")
            )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        active_ipr_fraction, ipr_status_label, latest_render_job, medium_dark_warning,
        missing_render_camera, render_running, render_status_lines, running_render_fraction,
    };
    use crate::editor::jobs::job_rows;
    use serde_json::json;

    #[test]
    fn dark_render_warns_only_with_a_substantial_camera_medium_depth() {
        assert_eq!(
            medium_dark_warning(Some(0.001), Some(28.)),
            Some("Camera medium may absorb everything (σ·distance ≈ 28.0)".into())
        );
        assert!(medium_dark_warning(Some(0.2), Some(28.)).is_none());
        assert!(medium_dark_warning(Some(0.001), Some(1.)).is_none());
        assert!(medium_dark_warning(None, Some(28.)).is_none());
    }

    #[test]
    fn cancel_is_live_only_while_a_final_render_runs() {
        assert!(!render_running(&json!({})));
        assert!(!render_running(&json!({"jobs": [
            {"kind": "render", "state": "succeeded"},
            {"kind": "ipr", "state": "running"},
        ]})));
        assert!(render_running(&json!({"jobs": [
            {"kind": "render", "state": "queued"},
        ]})));
    }

    #[test]
    fn missing_camera_notice_is_shown_only_for_an_active_scene_without_selection() {
        assert!(missing_render_camera(true, None));
        assert!(!missing_render_camera(true, Some("Camera")));
        assert!(!missing_render_camera(true, Some("None")));
        assert!(!missing_render_camera(false, None));
    }

    #[test]
    fn the_newest_render_job_is_the_status_source_and_others_never_show() {
        let jobs = json!({"jobs":[
            {"job_id":"c","kind":"project.preview","state":"running","phase":"export"},
            {"job_id":"b","kind":"project.open","state":"succeeded","phase":"done","root_path":"/p"},
            {"job_id":"a","kind":"render","state":"running","phase":"rendering","root_path":""}
        ]});
        let rows = job_rows(&jobs);
        assert_eq!(latest_render_job(&rows).unwrap().job_id, "a");
        let other_jobs = json!({"jobs":[
            {"job_id":"z","kind":"capture","state":"running","phase":"export"}
        ]});
        assert!(latest_render_job(&job_rows(&other_jobs)).is_none());
    }

    #[test]
    fn ipr_status_label_reports_the_live_state() {
        assert_eq!(ipr_status_label(false, &json!({})), None);
        assert_eq!(
            ipr_status_label(true, &json!({"active_job_id": "abc"})),
            Some("rendering…")
        );
        assert_eq!(
            ipr_status_label(true, &json!({"waiting_for_final_render": true})),
            Some("waiting for the final render")
        );
        assert_eq!(
            ipr_status_label(true, &json!({})),
            Some("watching for edits")
        );
    }

    #[test]
    fn render_status_lines_show_running_renders_with_elapsed_and_phase() {
        let jobs = json!({"jobs":[
            {"job_id":"c","kind":"capture","state":"running","phase":"export"},
            {"job_id":"a","kind":"render","state":"running","phase":"rendering","root_path":""}
        ]});
        let rows = job_rows(&jobs);
        let lines = render_status_lines(
            latest_render_job(&rows),
            "4s",
            None,
            Some(("42", true)),
            None,
            None,
        );
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            [
                "Project revision 42 · stale: project has changed",
                "Render · running (4s) · rendering",
            ]
        );
        assert!(!lines[0].spinner);
        assert!(lines[1].spinner);
        assert_eq!(lines[1].color, gpui_kit::rgb(0xe0b050));
    }

    #[test]
    fn render_status_lines_carry_the_failure_error_and_settled_outcomes() {
        let failed = json!({"jobs":[
            {"job_id":"a","kind":"render","state":"failed","phase":"failed","error":{"message":"renderer exited with code 1"}}
        ]});
        let lines = render_status_lines(
            latest_render_job(&job_rows(&failed)),
            "",
            None,
            Some(("7", false)),
            None,
            None,
        );
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            [
                "Project revision 7",
                "Render · failed · renderer exited with code 1"
            ]
        );
        assert_eq!(lines[1].color, gpui_kit::rgb(0xd99a74));

        let cancelled = json!({"jobs":[
            {"job_id":"a","kind":"render","state":"cancelled","phase":"cancelled"}
        ]});
        let lines = render_status_lines(
            latest_render_job(&job_rows(&cancelled)),
            "",
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Render · cancelled"]
        );

        let succeeded = json!({"jobs":[
            {"job_id":"a","kind":"render","state":"succeeded","phase":"complete"}
        ]});
        let lines = render_status_lines(
            latest_render_job(&job_rows(&succeeded)),
            "1m 20s",
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Render · succeeded (1m 20s)"]
        );
    }

    #[test]
    fn render_status_lines_place_the_ipr_note_below_the_render_and_spin_only_while_rendering() {
        let lines = render_status_lines(
            None,
            "",
            Some("rendering…"),
            Some(("42", false)),
            None,
            None,
        );
        assert!(lines.last().unwrap().spinner);
        let lines = render_status_lines(None, "", Some("watching for edits"), None, None, None);
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["watching for edits"]
        );
        assert!(!lines[0].spinner);
    }

    #[test]
    fn a_running_render_reports_the_received_pixel_fraction() {
        let jobs = json!({"jobs":[
            {"job_id":"a","kind":"render","state":"running","phase":"rendering","root_path":""}
        ]});
        let rows = job_rows(&jobs);
        let lines = render_status_lines(
            latest_render_job(&rows),
            "1m 12s",
            None,
            None,
            Some(0.379),
            None,
        );
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            // The whole percent is rounded down.
            ["Render · running (1m 12s) · rendering · 37% of pixels"]
        );
    }

    #[test]
    fn the_pixel_fraction_is_hidden_for_finished_renders_and_unknown_fractions() {
        let succeeded = json!({"jobs":[
            {"job_id":"a","kind":"render","state":"succeeded","phase":"complete"}
        ]});
        let lines = render_status_lines(
            latest_render_job(&job_rows(&succeeded)),
            "1m 20s",
            None,
            None,
            Some(0.379),
            None,
        );
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Render · succeeded (1m 20s)"]
        );

        let running = json!({"jobs":[
            {"job_id":"a","kind":"render","state":"running","phase":"rendering"}
        ]});
        let lines = render_status_lines(
            latest_render_job(&job_rows(&running)),
            "4s",
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Render · running (4s) · rendering"]
        );
    }

    #[test]
    fn the_ipr_rendering_line_reports_the_received_pixel_fraction() {
        let lines = render_status_lines(None, "", Some("rendering…"), None, None, Some(0.379));
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["rendering… · 37% of pixels"]
        );
        assert!(lines[0].spinner);
    }

    #[test]
    fn the_render_line_fraction_binds_to_its_own_running_job() {
        let stored = Some(("a".to_owned(), 0.379));
        let running = json!({"jobs":[
            {"job_id":"a","kind":"render","state":"running","phase":"rendering"}
        ]});
        let rows = job_rows(&running);
        assert_eq!(
            running_render_fraction(stored.as_ref(), latest_render_job(&rows)),
            Some(0.379)
        );
        // A finished render shows nothing, even for its own last frame.
        let succeeded = json!({"jobs":[
            {"job_id":"a","kind":"render","state":"succeeded","phase":"complete"}
        ]});
        let rows = job_rows(&succeeded);
        assert_eq!(
            running_render_fraction(stored.as_ref(), latest_render_job(&rows)),
            None
        );
        // A newer render running or queued over another job's image shows
        // nothing until its own first frame arrives.
        let queued = json!({"jobs":[
            {"job_id":"b","kind":"render","state":"queued","phase":"queued"}
        ]});
        let rows = job_rows(&queued);
        assert_eq!(
            running_render_fraction(stored.as_ref(), latest_render_job(&rows)),
            None
        );
        // An artifact without the field carries none.
        assert_eq!(
            running_render_fraction(None, latest_render_job(&rows)),
            None
        );
    }

    #[test]
    fn the_ipr_line_fraction_binds_to_the_active_ipr_job() {
        let stored = Some(("i".to_owned(), 0.379));
        assert_eq!(active_ipr_fraction(stored.as_ref(), Some("i")), Some(0.379));
        // IPR starting over a finished final render's partial must not
        // show that render's fraction.
        assert_eq!(
            active_ipr_fraction(Some(&("a".to_owned(), 0.379)), Some("i")),
            None
        );
        // IPR off, or an artifact without the field.
        assert_eq!(active_ipr_fraction(stored.as_ref(), None), None);
        assert_eq!(active_ipr_fraction(None, Some("i")), None);
    }
}
