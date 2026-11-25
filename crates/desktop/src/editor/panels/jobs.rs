//! Jobs panel: the job list with a Cancel for the ones still running.
use crate::backend::Command;
use crate::editor::EditorPanel;
use crate::editor::jobs::{
    JobStateIcon, elapsed_label, job_rows, state_color, state_icon, track_elapsed,
};
use gpui_kit::component::{Icon, Sizable, button::Button, spinner::Spinner};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Instant;

impl EditorPanel {
    pub(super) fn jobs_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = job_rows(&self.session.read(cx).jobs);
        let now = Instant::now();
        track_elapsed(&rows, &mut self.job_times, now);
        let mut list = div()
            .id("jobs-list")
            .size_full()
            .overflow_y_scroll()
            .text_xs();
        if rows.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py_2()
                    .text_color(rgb(0x8d93a1))
                    .child("No jobs yet."),
            );
        }
        for row in rows {
            let elapsed = self
                .job_times
                .get(&row.job_id)
                .map(|(start, done)| elapsed_label(done.unwrap_or_else(|| now - *start)))
                .unwrap_or_default();
            let color = state_color(&row.state);
            // A small icon left of the state text, in the state's colour:
            // active states spin, settled states show their result.
            let icon = match state_icon(&row.state) {
                JobStateIcon::Spinner => Spinner::new()
                    .color(color.into())
                    .xsmall()
                    .into_any_element(),
                JobStateIcon::Check => Icon::new(assets::IconName::Check)
                    .xsmall()
                    .text_color(color)
                    .into_any_element(),
                JobStateIcon::Cross => Icon::new(assets::IconName::X)
                    .xsmall()
                    .text_color(color)
                    .into_any_element(),
                JobStateIcon::CircleSlash => Icon::new(assets::IconName::CircleSlash)
                    .xsmall()
                    .text_color(color)
                    .into_any_element(),
            };
            let job_id = row.job_id.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("job-{}", row.job_id)))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(rgb(0x30333a))
                    .child(div().w(px(110.)).truncate().child(row.kind.clone()))
                    .child(
                        div()
                            .w(px(80.))
                            .text_color(color)
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(icon)
                            .child(row.state.clone()),
                    )
                    .child(div().w(px(110.)).truncate().child(row.phase.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(0x8d93a1))
                            .child(row.detail.clone()),
                    )
                    .child(div().w(px(56.)).text_color(rgb(0x8d93a1)).child(elapsed))
                    .when(row.active && row.state != "cancelling", |this| {
                        this.child(
                            Button::new(SharedString::from(format!("job-cancel-{}", row.job_id)))
                                .label("Cancel")
                                .small()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let job_id = job_id.clone();
                                    this.session.update(cx, |session, cx| {
                                        session.send(Command::CancelJob(job_id), cx)
                                    });
                                })),
                        )
                    }),
            );
        }
        list.into_any_element()
    }
}
