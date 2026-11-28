//! Small modal dialogs: a text field, the New Project fields, or a
//! yes/no confirmation.
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{WindowExt as _, button::Button, button::ButtonVariants as _};
use gpui_kit::*;
use std::rc::Rc;

type OnText = Rc<dyn Fn(String, &mut Window, &mut App)>;
type OnConfirm = Rc<dyn Fn(&mut Window, &mut App)>;

/// Ask for one line of text. `on_ok` runs with the trimmed value when the
/// user presses OK or Enter; Cancel and Escape send nothing.
pub fn text(
    window: &mut Window,
    cx: &mut App,
    title: impl Into<SharedString>,
    initial: &str,
    on_ok: impl Fn(String, &mut Window, &mut App) + 'static,
) {
    let title = title.into();
    let initial = initial.to_owned();
    let state = cx.new(|cx| InputState::new(window, cx).default_value(initial));
    let on_ok: OnText = Rc::new(on_ok);
    {
        let state = state.clone();
        let on_ok = on_ok.clone();
        window
            .subscribe(
                &state.clone(),
                cx,
                move |_, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        let value = state.read(cx).value().trim().to_owned();
                        window.close_dialog(cx);
                        on_ok(value, window, cx);
                    }
                },
            )
            .detach();
    }
    window.open_dialog(cx, move |dialog, window, cx| {
        state.update(cx, |state, cx| state.focus(window, cx));
        let ok_state = state.clone();
        let ok = on_ok.clone();
        dialog
            .title(title.clone())
            .child(Input::new(&state))
            .footer(
                DialogFooter::new()
                    .child(Button::new("prompt-cancel").label("Cancel").on_click(
                        |_, window, cx| {
                            window.close_dialog(cx);
                        },
                    ))
                    .child(Button::new("prompt-ok").label("OK").primary().on_click(
                        move |_, window, cx| {
                            let value = ok_state.read(cx).value().trim().to_owned();
                            window.close_dialog(cx);
                            ok(value, window, cx);
                        },
                    )),
            )
    });
}

/// Ask before a destructive action; `on_confirm` runs only on the confirm
/// button.
pub fn confirm(
    window: &mut Window,
    cx: &mut App,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    confirm_label: &'static str,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    let title = title.into();
    let description = description.into();
    let on_confirm: OnConfirm = Rc::new(on_confirm);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let on_confirm = on_confirm.clone();
        alert
            .title(title.clone())
            .description(description.clone())
            .footer(
                DialogFooter::new()
                    .justify_center()
                    .child(Button::new("confirm-cancel").label("Cancel").on_click(
                        |_, window, cx| {
                            window.close_dialog(cx);
                        },
                    ))
                    .child(
                        Button::new("confirm-ok")
                            .label(confirm_label)
                            .danger()
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                on_confirm(window, cx);
                            }),
                    ),
            )
    });
}

/// New Project: a destination folder and a title. The Folder
/// row keeps a typed-path field beside the same picker Open Project
/// uses — the picker is preferred, the field keeps the dialog usable
/// where no portal picker exists — and choosing a folder pre-titles an
/// untouched Title field with the folder's name.
/// `on_create` runs with both trimmed values when Create is pressed;
/// Cancel and Escape send nothing.
pub fn new_project(
    window: &mut Window,
    cx: &mut App,
    on_create: impl Fn(String, String, &mut Window, &mut App) + 'static,
) {
    let folder = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("New or empty folder; a used one gets a subfolder named after the title")
    });
    let title = cx.new(|cx| InputState::new(window, cx).placeholder("Project title"));
    // The Create contract mirrors `confirm`'s callback shape a few lines
    // above, shared with the Enter bindings and `create` below.
    #[allow(clippy::type_complexity)]
    let on_create: Rc<dyn Fn(String, String, &mut Window, &mut App)> = Rc::new(on_create);
    // Enter in either field confirms, like prompt::text's OK.
    for field in [&folder, &title] {
        let folder = folder.clone();
        let title = title.clone();
        let on_create = on_create.clone();
        window
            .subscribe(field, cx, move |_, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    create(&on_create, &folder, &title, window, cx);
                }
            })
            .detach();
    }
    // The builder runs on every render: focus Folder only the first time,
    // or a click into Title is pulled straight back.
    let focused = std::cell::Cell::new(false);
    window.open_dialog(cx, move |dialog, window, cx| {
        if !focused.replace(true) {
            folder.update(cx, |state, cx| state.focus(window, cx));
        }
        let folder_pick = folder.clone();
        let title_pick = title.clone();
        let create_folder = folder.clone();
        let create_title = title.clone();
        let create_ok = on_create.clone();
        dialog
            .title("New Project")
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pb_1()
                    .child(div().text_color(rgb(0x9098a6)).child("Folder"))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap_1()
                            .child(Input::new(&folder).w_full())
                            .child(Button::new("new-project-choose").label("Choose…").on_click(
                                move |_, window, cx| {
                                    choose_folder(window, cx, &folder_pick, &title_pick);
                                },
                            )),
                    )
                    .child(div().pt_1().text_color(rgb(0x9098a6)).child("Title"))
                    .child(Input::new(&title).w_full()),
            )
            .footer(
                DialogFooter::new()
                    .child(Button::new("new-project-cancel").label("Cancel").on_click(
                        |_, window, cx| {
                            window.close_dialog(cx);
                        },
                    ))
                    .child(
                        Button::new("new-project-create")
                            .label("Create")
                            .primary()
                            .on_click(move |_, window, cx| {
                                create(&create_ok, &create_folder, &create_title, window, cx);
                            }),
                    ),
            )
    });
}

/// Read both fields, close the dialog, and hand the trimmed values over.
#[allow(clippy::type_complexity)]
fn create(
    on_create: &Rc<dyn Fn(String, String, &mut Window, &mut App)>,
    folder: &Entity<InputState>,
    title: &Entity<InputState>,
    window: &mut Window,
    cx: &mut App,
) {
    let folder = folder.read(cx).value().trim().to_owned();
    let title = title.read(cx).value().trim().to_owned();
    window.close_dialog(cx);
    on_create(folder, title, window, cx);
}

/// The Folder row's picker: a directory-only prompt like Open Project.
fn choose_folder(
    window: &mut Window,
    cx: &mut App,
    folder: &Entity<InputState>,
    title: &Entity<InputState>,
) {
    let picker = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Choose the new project folder".into()),
    });
    let folder = folder.clone();
    let title = title.clone();
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(paths))) = picker.await
                && let Some(path) = paths.into_iter().next()
            {
                let text = path.display().to_string();
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let _ = folder.update_in(cx, |state, window, cx| {
                    state.set_value(text, window, cx);
                });
                let _ = title.update_in(cx, |state, window, cx| {
                    if state.value().trim().is_empty() && !name.is_empty() {
                        state.set_value(name, window, cx);
                    }
                });
            }
        })
        .detach();
}
