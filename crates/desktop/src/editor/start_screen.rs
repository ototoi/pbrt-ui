//! The Start screen shown instead of the Dock while no document is open.
use super::Workspace;
use super::menu::{NewProject, OpenPbrtScene, OpenProject, OpenRecent};
use super::recent::{self, RecentKind};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::{Sizable, button::Button};
use gpui_kit::*;
use std::time::SystemTime;

impl Workspace {
    pub(super) fn start_screen(&self, cx: &mut Context<Self>) -> AnyElement {
        let now = SystemTime::now();
        let mut rows = div().flex().flex_col().gap_1().w(px(560.));
        if self.recent.is_empty() {
            rows = rows.child(
                div()
                    .text_xs()
                    .text_color(rgb(0x8d93a1))
                    .child("Projects and scenes you open appear here."),
            );
        }
        for (index, entry) in self.recent.iter().enumerate() {
            let missing = !entry.path.exists();
            let (icon, title) = match entry.kind {
                RecentKind::Project => ("▣", entry.title.clone()),
                RecentKind::Pbrt => (
                    "◇",
                    entry
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| entry.title.clone()),
                ),
            };
            let detail = if missing {
                "(not found)".to_owned()
            } else if entry.kind == RecentKind::Pbrt {
                format!("(viewer)  {}", recent::age(&entry.opened_at, now))
            } else {
                recent::age(&entry.opened_at, now)
            };
            let path = entry.path.clone();
            let mut row = div()
                .id(SharedString::from(format!("recent-{index}")))
                .flex()
                .items_center()
                .gap_3()
                .px_2()
                .py_1()
                .rounded_sm()
                .text_sm()
                .child(div().w(px(16.)).child(icon))
                .child(div().w(px(150.)).truncate().child(title))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(0x8d93a1))
                        .child(entry.path.display().to_string()),
                )
                .child(div().text_xs().text_color(rgb(0x8d93a1)).child(detail));
            if missing {
                row = row.opacity(0.6).child(
                    Button::new(SharedString::from(format!("recent-remove-{index}")))
                        .label("Remove")
                        .small()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.forget_recent(&path, cx);
                        })),
                );
            } else {
                row = row
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x30333a)))
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(OpenRecent { index }), cx)
                    });
            }
            rows = rows.child(row);
        }
        div()
            .id("start-screen")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(rgb(0x25272d))
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::BOLD)
                    .child("pbrt-ui"),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        Button::new("start-new-project")
                            .label("New Project…")
                            .primary()
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(NewProject), cx)
                            }),
                    )
                    .child(
                        Button::new("start-open-project")
                            .label("Open Project…")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenProject), cx)
                            }),
                    )
                    .child(
                        Button::new("start-open-scene")
                            .label("Open PBRT Scene…")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenPbrtScene), cx)
                            }),
                    ),
            )
            .child(
                div()
                    .w(px(560.))
                    .pt_2()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0x8d93a1))
                    .child("Recent"),
            )
            .child(rows)
            .into_any_element()
    }
}
