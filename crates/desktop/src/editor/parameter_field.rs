//! One typed parameter row, shared by the
//! Inspector and the node editor: number, color, spectrum name, checkbox
//! and text fields edit in place; texture, material and file references
//! are links;
//! everything else keeps the JSON form. An undeclared table row
//! shows its default greyed and declares itself when edited; Reset removes
//! the declaration so r4's default applies again.
use super::Session;
use super::menu::RevealItem;
use super::parameter_form::{self, ParameterForm};
use super::parameter_rows::{self, Field, Row};
use super::parameter_values::{self, Table};
use crate::backend::{Command, Follow};
use gpui::{App, Context, Entity, IntoElement, ParentElement, Render, Rgba, Styled, Window};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{
    Input, InputEvent, InputState, NumberInput, NumberInputEvent, StepAction,
};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use std::rc::Rc;
use std::time::Duration;

pub struct ParameterField {
    session: Entity<Session>,
    target: Value,
    project: Value,
    row: Row,
    /// The colour dialog behind a spectrum-capable row's colour box.
    dialog: Option<Entity<super::color_dialog::ColourEdit>>,
    /// Names for links (texture, materials, asset).
    names: Vec<(String, String)>,
    /// 's Connect menu: this revision's Contents items, shared by the
    /// page's fields (the textures of the row's lineage are listed when the
    /// menu opens, not per field per revision), and the implementations a
    /// new texture can take.
    items: Rc<Value>,
    create: Vec<String>,
    inputs: Vec<Entity<InputState>>,
    /// The texts shown when the field was built, to tell unsent typing.
    initial: Vec<String>,
    draft_history: bool,
    submitted_texts: Option<Vec<String>>,
    /// A declared value the typed fields cannot show.
    json: Option<Entity<ParameterForm>>,
    /// Stable across revisions: `parameter:<target id>:<name>`.
    form: String,
    message: String,
    step_generation: u64,
    step_baseline: Option<i64>,
}

fn texts(field: &Field) -> Vec<String> {
    match field {
        Field::Number { value, .. } => vec![value.clone()],
        Field::Rgb(xyz) | Field::Vector(xyz) => xyz.iter().map(f64::to_string).collect(),
        Field::Text(text) => vec![text.clone()],
        _ => Vec::new(),
    }
}

impl ParameterField {
    /// Keep the input entities (and their focus) across confirmed revisions.
    pub fn refresh_texts(
        &mut self,
        project: &Value,
        row: &Row,
        scale: Option<Row>,
        items: Rc<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let initial = texts(&row.field);
        if initial.is_empty()
            || initial.len() != self.inputs.len()
            || std::mem::discriminant(&self.row.field) != std::mem::discriminant(&row.field)
        {
            return false;
        }
        for (input, value) in self.inputs.iter().zip(&initial) {
            input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        self.step_generation = self.step_generation.wrapping_add(1);
        self.project = project.clone();
        self.row = row.clone();
        self.items = items;
        self.initial = initial;
        self.draft_history = false;
        self.submitted_texts = None;
        self.message.clear();
        // The colour dialog behind a spectrum row follows the
        // confirmed revision too, adopting only the fields that changed.
        if let Some(dialog) = &self.dialog {
            dialog.update(cx, |dialog, cx| {
                dialog.refresh(project, row, scale, self.items.clone(), window, cx)
            });
        }
        cx.notify();
        true
    }

    /// (gpui constructors take the window and the context by necessity;
    /// the construction inputs are the six before them.)
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: Entity<Session>,
        target: Value,
        project: Value,
        page: &Value,
        row: Row,
        items: Rc<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let form = format!(
            "parameter:{}:{}",
            target["id"].as_str().unwrap_or_default(),
            row.name
        );
        // A spectrum-capable row's colour box opens the colour
        // dialog; the dialog entity holds its drafts (the inputs below
        // stay the row's own typed fields).
        let dialog = (parameter_rows::spectrum_view(&row) != parameter_rows::SpectrumView::Typed)
            .then(|| {
                cx.new(|cx| {
                    super::color_dialog::ColourEdit::new(
                        session.clone(),
                        target.clone(),
                        project.clone(),
                        page,
                        row.clone(),
                        items.clone(),
                        window,
                        cx,
                    )
                })
            });
        let initial = texts(&row.field);
        let inputs = initial
            .iter()
            .map(|text| {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
                // Enter applies the whole row (all three channels of a
                // color at once). Focus changes are not relied on: the
                // inputs here do not report them, and applying per channel
                // would rebuild the row under the cursor.
                cx.subscribe_in(
                    &input,
                    window,
                    |this, _state, event: &InputEvent, _window, cx| {
                        if matches!(event, InputEvent::Change) {
                            this.draft_history = true;
                        }
                        // Enter commits typed text, as before. Blur only settles a run of
                        // spin-box steps; typed text still waits for Enter.
                        if matches!(event, InputEvent::PressEnter { .. }) && this.is_modified(cx) {
                            this.apply_texts(cx);
                        }
                        if matches!(event, InputEvent::Blur) && this.is_integer_single() {
                            this.finish_steps(cx);
                        }
                    },
                )
                .detach();
                if matches!(row.field, Field::Number { integer: true, .. }) {
                    input.update(cx, |state, cx| state.set_step(None, window, cx));
                }
                cx.subscribe_in(
                    &input,
                    window,
                    |this, state, event: &NumberInputEvent, window, cx| {
                        let NumberInputEvent::Step(action) = event;
                        if this.is_integer_single() {
                            this.step_integer(state, *action, window, cx);
                        }
                    },
                )
                .detach();
                cx.observe(&input, |_, _, cx| cx.notify()).detach();
                input
            })
            .collect();
        let json = (row.field == Field::Json
            && ParameterForm::supports(&row.parameter)
            // Arrays of two or more values open the values table instead.
            && parameter_values::table(&row).is_none())
        .then(|| {
            cx.new(|cx| {
                ParameterForm::new(
                    session.clone(),
                    target.clone(),
                    project.clone(),
                    &row.parameter,
                    window,
                    cx,
                )
            })
        });
        let ids = match &row.field {
            Field::Texture(id) | Field::Asset(id) | Field::SpectrumData(id) => vec![id.clone()],
            Field::Materials(ids) => ids.clone(),
            _ => Vec::new(),
        };
        let names = ids
            .into_iter()
            .map(|id| {
                let name = super::components::item_name(&items, &id);
                (id, name)
            })
            .collect();
        let create = row
            .texture
            .as_ref()
            .filter(|_| parameter_rows::connectable(&target, &row))
            .map(|lineage| {
                super::resource_menu::implementations(
                    &session.read(cx).resource_schema[format!("texture_{lineage}")],
                )
            })
            .unwrap_or_default();
        Self {
            session,
            target,
            project,
            row,
            dialog,
            names,
            items,
            create,
            inputs,
            initial,
            draft_history: false,
            submitted_texts: None,
            json,
            form,
            message: String::new(),
            step_generation: 0,
            step_baseline: None,
        }
    }

    pub fn form_id(&self) -> &str {
        &self.form
    }

    pub fn name(&self) -> &str {
        &self.row.name
    }

    /// The user typed something that was not sent.
    pub fn has_unsent_changes(&self, cx: &App) -> bool {
        let texts = self
            .inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect::<Vec<_>>();
        self.is_modified(cx) && self.submitted_texts.as_ref() != Some(&texts)
    }

    pub fn is_modified(&self, cx: &App) -> bool {
        self.inputs
            .iter()
            .zip(&self.initial)
            .any(|(input, initial)| input.read(cx).value() != initial.as_str())
            || self
                .json
                .as_ref()
                .is_some_and(|json| json.read(cx).is_modified(cx))
    }

    fn send(&mut self, edit: Value, cx: &mut Context<Self>) {
        self.send_all(vec![edit], cx)
    }

    /// `edits` as one transaction (one undo step).
    fn send_all(&mut self, edits: Vec<Value>, cx: &mut Context<Self>) {
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
        let form = self.form.clone();
        self.session.update(cx, |session, cx| {
            session.edit_feedback.submitted(&form);
            session.send(
                Command::Apply {
                    project,
                    edits,
                    follow: Follow::Nothing,
                    form: Some(form.clone()),
                },
                cx,
            )
        });
        self.message.clear();
        cx.notify();
    }

    fn apply_texts(&mut self, cx: &mut Context<Self>) {
        let texts = self
            .inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect::<Vec<_>>();
        self.apply_values(&texts, cx);
    }

    fn is_integer_single(&self) -> bool {
        matches!(self.row.field, Field::Number { integer: true, .. })
    }

    fn integer_minimum(&self) -> Option<i64> {
        // Sample counts need at least one; maxdepth 0 is valid in PBRT (emitted
        // light only).
        match self.row.name.as_str() {
            "pixelsamples" | "nsamples" => Some(1),
            "maxdepth" => Some(0),
            _ => None,
        }
    }

    fn step_integer(
        &mut self,
        state: &Entity<InputState>,
        action: StepAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = state.read(cx).value().parse::<i64>().unwrap_or(0);
        self.step_baseline.get_or_insert(current);
        let next = stepped_integer(current, action, self.integer_minimum());
        state.update(cx, |input, cx| {
            input.set_value(next.to_string(), window, cx)
        });
        self.step_generation = self.step_generation.wrapping_add(1);
        let generation = self.step_generation;
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let _ = entity.update(cx, |this, cx| {
                if this.step_generation == generation {
                    this.finish_steps(cx);
                }
            });
        })
        .detach();
    }

    fn finish_steps(&mut self, cx: &mut Context<Self>) {
        self.step_generation = self.step_generation.wrapping_add(1);
        let current = self.inputs[0].read(cx).value().parse::<i64>().ok();
        if take_step_commit(&mut self.step_baseline, current) {
            self.apply_texts(cx);
        }
    }

    fn apply_values(&mut self, texts: &[String], cx: &mut Context<Self>) {
        if self.is_integer_single()
            && let (Some(minimum), Some(value)) = (
                self.integer_minimum(),
                texts.first().and_then(|value| value.parse::<i64>().ok()),
            )
            && value < minimum
        {
            self.message = format!("Enter an integer of at least {minimum}");
            cx.notify();
            return;
        }
        match parameter_rows::values(&self.row.field, texts) {
            Some(values) => {
                let edit = parameter_rows::set_edit(&self.target, &self.row, values);
                self.send(edit, cx);
            }
            None => {
                self.message = "Enter a number".into();
                cx.notify();
            }
        }
    }

    fn link(&self, index: usize) -> AnyElement {
        let (id, name) = self.names[index].clone();
        div()
            .id(SharedString::from(format!("{}-link-{index}", self.form)))
            .text_color(rgb(0x7fb2f0))
            .cursor_pointer()
            .truncate()
            .hover(|style| style.underline())
            .child(name)
            .on_click(move |_, window, cx| {
                window.dispatch_action(Box::new(RevealItem { id: id.clone() }), cx)
            })
            .into_any_element()
    }

    /// `button` with a menu that sets this row to one of `values`.
    fn pick(&self, button: Button, values: &[String], cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let values = values.to_vec();
        button
            .xsmall()
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu.scrollable(true).max_h(px(320.));
                for value in &values {
                    let (this, value) = (this.clone(), value.clone());
                    menu = menu.item(
                        PopupMenuItem::new(parameter_rows::choice_label(&value)).on_click(
                            move |_, _, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    this.apply_values(std::slice::from_ref(&value), cx)
                                });
                            },
                        ),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// The one colour box of a spectrum-capable row: the wire's
    /// approximate colour (`project.spectrum_color` behind the backend,
    /// asked at most once per visible row per revision), the human
    /// `source` string on hover. An unset row paints its table default
    /// greyed (the row's own opacity) and asks nothing; while a declared
    /// row's answer is on its way or was refused the box draws hollow
    /// with the reason on hover. The box is the trigger
    /// of the colour dialog; the row's role picks the read: the
    /// reflectance box keeps its albedo, lights and unbounded eta/k
    /// show chromaticity.
    fn colour_box(&self, cx: &mut Context<Self>) -> Button {
        // Only a declared row resolves a colour: unset rows keep their
        // greyed default.
        let answer = self
            .row
            .declared
            .then(|| {
                self.session.update(cx, |session, cx| {
                    session.parameter_swatch(
                        &self.target,
                        &self.row.name,
                        parameter_rows::row_role(&self.row).as_deref(),
                        cx,
                    )
                })
            })
            .flatten();
        let (paint, tooltip) = match &answer {
            Some(answer) if answer.get("error").is_none() => {
                let channel = |index: usize| {
                    answer["rgb"]
                        .as_array()
                        .and_then(|values| values.get(index))
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or_default()
                };
                (
                    Some(Rgba {
                        r: channel(0).clamp(0., 1.) as f32,
                        g: channel(1).clamp(0., 1.) as f32,
                        b: channel(2).clamp(0., 1.) as f32,
                        a: 1.,
                    }),
                    answer["source"].as_str().unwrap_or_default().to_owned(),
                )
            }
            // A refused read: the reason rides the tooltip, the box stays
            // hollow — no colour resolved.
            Some(answer) => (
                None,
                answer["error"].as_str().unwrap_or_default().to_owned(),
            ),
            // A declared row while its answer is on its way.
            None if self.row.declared => (None, "resolving the colour…".to_owned()),
            // An unset row's greyed table default: an rgb triple paints
            // as such, anything else draws hollow (the filled
            // muted dark read as "0").
            None => {
                // A hint means no constant default: the zeros are only a
                // placeholder, so draw hollow.
                let paint = match &self.row.field {
                    Field::Rgb(xyz) if self.row.hint.is_empty() => Some(Rgba {
                        r: xyz[0].clamp(0., 1.) as f32,
                        g: xyz[1].clamp(0., 1.) as f32,
                        b: xyz[2].clamp(0., 1.) as f32,
                        a: 1.,
                    }),
                    _ => None,
                };
                (paint, "not set — click to give a colour".to_owned())
            }
        };
        // The swatch is a child div: gpui-kit's custom button variant mixes
        // its colour 80% toward transparent, which painted every box faint.
        // Nothing resolved paints the shared hollow look.
        let swatch = if let Some(paint) = paint {
            div().size_full().rounded_sm().bg(paint)
        } else {
            // Clipped to the box's rounding so the hollow outline keeps
            // the fill's silhouette.
            div()
                .size_full()
                .rounded_sm()
                .overflow_hidden()
                .child(super::components::unset_swatch())
        };
        Button::new(SharedString::from(format!("{}-colour-box", self.form)))
            .ghost()
            .p_0()
            .w(px(18.))
            .h(px(16.))
            .rounded_sm()
            .child(swatch)
            .tooltip(tooltip)
    }

    fn value(&self, cx: &mut Context<Self>) -> AnyElement {
        let input = |input: &Entity<InputState>| Input::new(input).xsmall().into_any_element();
        // A spectrum-capable row — unbounded eta/k included — shows
        // one approximate colour box (the curve's chromaticity for
        // unbounded values, a colour "as such" would lie about), the
        // box opens the same colour dialog the rgb fields do. A short
        // muted label beside it names the value (item 9: the
        // resource, the name, the kelvin), truncated with the full text
        // on hover.
        // A row rendered as a texture connection is not a colour value:
        // it keeps its link.
        if parameter_rows::spectrum_view(&self.row) == parameter_rows::SpectrumView::Colour
            && !matches!(self.row.field, Field::Texture(_) | Field::Materials(_))
        {
            let label = parameter_rows::value_label(&self.row, &self.items);
            let box_in_row = |this: &Self, cx: &mut Context<Self>| -> AnyElement {
                let Some(dialog) = this.dialog.clone() else {
                    return this.colour_box(cx).into_any_element();
                };
                // A value-type change rebuilds this field,
                // and the popover's open state is gpui element state keyed
                // under the field's entity — it dies with the old one. The
                // session keeps the open flag by form id, this controlled
                // `open` re-opens the rebuilt field's dialog, and the
                // popover reports every announced dismissal to clear it.
                let open = this.session.read(cx).colour_dialog_open(&this.form);
                let (session, form) = (this.session.clone(), this.form.clone());
                Popover::new(SharedString::from(format!("{}-colour-dialog", this.form)))
                    .trigger(this.colour_box(cx))
                    .open(open)
                    .on_open_change(move |is_open, _, cx| {
                        session.update(cx, |session, _| {
                            session.set_colour_dialog_open(&form, *is_open);
                        });
                    })
                    .content(move |_, _, _| dialog.clone())
                    .into_any_element()
            };
            let mut row = div().flex().items_center().gap_1().min_w_0();
            row = row.child(box_in_row(self, cx));
            if let Some(label) = label {
                row = row.child(
                    div()
                        .id(SharedString::from(format!("{}-value-label", self.form)))
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(0x9098a6))
                        .child(label.clone())
                        .tooltip(super::components::text_tooltip(label)),
                );
            }
            return row.into_any_element();
        }
        match &self.row.field {
            // An undeclared mix row arrives as an empty text field.
            Field::Materials(ids) if self.is_mix_materials() => self.mix_materials(ids, cx),
            _ if self.is_mix_materials() => self.mix_materials(&[], cx),
            // A string with fixed choices picks one of them.
            Field::Text(current) if !self.row.choices.is_empty() => self.pick(
                Button::new(SharedString::from(format!("{}-choice", self.form)))
                    .label(parameter_rows::choice_label(current)),
                &self.row.choices,
                cx,
            ),
            // Free text with suggestions: type, or pick a common value.
            Field::Text(_) if !self.row.suggestions.is_empty() => div()
                .flex()
                .gap_1()
                .items_center()
                .child(div().flex_1().min_w_0().child(input(&self.inputs[0])))
                .child(
                    self.pick(
                        Button::new(SharedString::from(format!("{}-suggest", self.form)))
                            .icon(gpui_kit::assets::IconName::ChevronDown)
                            .tooltip("Common values"),
                        &self.row.suggestions,
                        cx,
                    ),
                )
                .into_any_element(),
            Field::Number { integer: true, .. } => NumberInput::new(&self.inputs[0])
                .xsmall()
                .into_any_element(),
            Field::Number { .. } | Field::Text(_) => input(&self.inputs[0]),
            Field::Vector(_) => div()
                .flex()
                .gap_1()
                .items_center()
                .children(
                    self.inputs
                        .iter()
                        .map(|i| div().flex_1().min_w_0().child(input(i))),
                )
                .into_any_element(),
            Field::Rgb(_) => {
                let channel = |index: usize| {
                    self.inputs[index]
                        .read(cx)
                        .value()
                        .trim()
                        .parse::<f64>()
                        .unwrap_or(0.)
                        .clamp(0., 1.)
                };
                let swatch = Rgba {
                    r: channel(0) as f32,
                    g: channel(1) as f32,
                    b: channel(2) as f32,
                    a: 1.,
                };
                div()
                    .flex()
                    .gap_1()
                    .items_center()
                    .child(div().size(px(16.)).flex_shrink_0().rounded_sm().bg(swatch))
                    .children(
                        self.inputs
                            .iter()
                            .map(|i| div().flex_1().min_w_0().child(input(i))),
                    )
                    .into_any_element()
            }
            Field::Bool(checked) => {
                let checked = *checked;
                Checkbox::new(SharedString::from(format!("{}-check", self.form)))
                    .checked(checked)
                    .on_click(cx.listener(move |this, value: &bool, _, cx| {
                        this.apply_values(&[value.to_string()], cx)
                    }))
                    .into_any_element()
            }
            Field::Spectrum(name) => {
                let this = cx.entity().downgrade();
                let spectra = self.session.read(cx).resource_schema["spectra"].clone();
                Button::new(SharedString::from(format!("{}-spectrum", self.form)))
                    .label(name.clone())
                    .xsmall()
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = menu.scrollable(true).max_h(px(320.));
                        for name in spectra.as_array().into_iter().flatten() {
                            let Some(name) = name.as_str().map(str::to_owned) else {
                                continue;
                            };
                            let this = this.clone();
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                                move |_, _, cx| {
                                    let _ = this.update(cx, |this, cx| {
                                        this.apply_values(std::slice::from_ref(&name), cx)
                                    });
                                },
                            ));
                        }
                        menu
                    })
                    .into_any_element()
            }
            Field::Texture(id) => {
                let id = id.clone();
                div()
                    .flex()
                    .gap_1()
                    .items_center()
                    .child(div().text_color(rgb(0x9098a6)).child("◀"))
                    .child(div().min_w_0().child(self.link(0)))
                    .child(
                        Button::new(SharedString::from(format!("{}-open", self.form)))
                            .icon(gpui_kit::assets::IconName::SquarePen)
                            .xsmall()
                            .tooltip("Open in Editor")
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(super::menu::OpenInEditor { id: id.clone() }),
                                    cx,
                                )
                            }),
                    )
                    .into_any_element()
            }
            Field::Asset(_) => self.link(0),
            // A SpectrumData reference is a link to the curve
            // resource, with the way to open its editor — never the
            // JSON fallback.
            Field::SpectrumData(id) => {
                let shown = id.clone();
                div()
                    .flex()
                    .gap_1()
                    .items_center()
                    .child(div().min_w_0().child(self.link(0)))
                    .child(
                        Button::new(SharedString::from(format!("{}-open", self.form)))
                            .icon(gpui_kit::assets::IconName::SquarePen)
                            .xsmall()
                            .tooltip("Open in Editor")
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(super::menu::OpenInEditor { id: shown.clone() }),
                                    cx,
                                )
                            }),
                    )
                    .into_any_element()
            }
            Field::Materials(ids) => div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children((0..ids.len()).map(|index| self.link(index)))
                .into_any_element(),
            Field::Unset => div().into_any_element(),
            Field::Json => match parameter_values::table(&self.row) {
                // The array values review: the summary opens the table.
                Some(table) => self.array_summary(table, cx),
                None => match &self.json {
                    Some(json) => json.clone().into_any_element(),
                    None => div()
                        .truncate()
                        .text_color(rgb(0xc4c8d0))
                        .child(format!(
                            "{} (read-only preview)",
                            super::components::values_text(&self.row.parameter["values"])
                        ))
                        .into_any_element(),
                },
            },
        }
    }

    fn is_mix_materials(&self) -> bool {
        self.row.implementation == "mix" && self.row.name == "materials"
    }

    fn mix_materials(&self, ids: &[String], cx: &mut Context<Self>) -> AnyElement {
        let target_id = self.target["id"].as_str().unwrap_or_default().to_owned();
        let choices = self
            .items
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(id, item)| item["resource_kind"] == "material" && id.as_str() != target_id)
            .map(|(id, item)| (item["name"].as_str().unwrap_or(id).to_owned(), id.clone()))
            .collect::<Vec<_>>();
        let this = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .children((0..2).map(|index| {
                let this = this.clone();
                let label = ids
                    .get(index)
                    .and_then(|id| self.items.get(id))
                    .and_then(|item| item["name"].as_str())
                    .unwrap_or("Choose material")
                    .to_owned();
                let choices = choices.clone();
                let existing = ids.to_vec();
                let target = self.target.clone();
                let button = Button::new(SharedString::from(format!(
                    "{}-material-{index}",
                    self.form
                )))
                .label(format!("Material {}: {label}", index + 1))
                .xsmall();
                button.dropdown_menu(move |mut menu, _, _| {
                    menu = menu.scrollable(true).max_h(px(320.));
                    for (name, id) in choices.iter() {
                        let this = this.clone();
                        let existing = existing.clone();
                        let target = target.clone();
                        let id = id.clone();
                        menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                            move |_, _, cx| {
                                let edit = parameter_rows::mix_material_edit(
                                    &target, &existing, index, &id,
                                );
                                let _ = this.update(cx, |this, cx| this.send(edit, cx));
                            },
                        ));
                    }
                    menu
                })
            }))
            .into_any_element()
    }

    /// Arrays open in their dedicated editor; the Inspector only shows metadata.
    fn array_summary(&self, table: Table, cx: &mut Context<Self>) -> AnyElement {
        let target = self.target.clone();
        let name = self.row.name.clone();
        let read_only = self.session.read(cx).document().is_read_only();
        div()
            .flex()
            .gap_1()
            .items_center()
            .min_w_0()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(0xc4c8d0))
                    .child(table.summary()),
            )
            .child(
                Button::new(format!("{}-open-array-editor", self.form))
                    .label(if read_only { "View…" } else { "Edit…" })
                    .xsmall()
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(
                            Box::new(super::menu::OpenArrayEditor {
                                target: target.clone(),
                                name: name.clone(),
                                table: table.clone(),
                            }),
                            cx,
                        )
                    }),
            )
            .into_any_element()
    }
}

fn stepped_integer(value: i64, action: StepAction, minimum: Option<i64>) -> i64 {
    let delta = match action {
        StepAction::Increment => 1_i64,
        StepAction::Decrement => -1_i64,
    };
    value.saturating_add(delta).max(minimum.unwrap_or(i64::MIN))
}

fn take_step_commit(baseline: &mut Option<i64>, current: Option<i64>) -> bool {
    baseline
        .take()
        .is_some_and(|baseline| Some(baseline) != current)
}

impl Render for ParameterField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A connected row disconnects instead (the same removal, named for
        // what it does); the Connect menu sits on every texture input,
        // and on every spectrum-capable row, where it also lists the
        // project's Spectrum Data — the same declaration edits the colour
        // dialog's Spectrum tab applies.
        let connected = matches!(self.row.field, Field::Texture(_))
            && parameter_rows::connectable(&self.target, &self.row);
        // A shape's alpha mask keeps its row: disconnecting goes back to
        // the opaque 1 instead of removing the declaration.
        let reset = connected
            .then(|| super::component_menu::alpha_mask_disconnect(&self.target, &self.row.name))
            .flatten()
            .or_else(|| parameter_rows::removal_edit(&self.target, &self.row).map(|e| vec![e]));
        let takes_spectrum_data =
            parameter_rows::spectrum_view(&self.row) != parameter_rows::SpectrumView::Typed;
        let connect = (parameter_rows::connectable(&self.target, &self.row) || takes_spectrum_data)
            .then(|| {
                let this = cx.entity().downgrade();
                let (target, row) = (self.target.clone(), self.row.clone());
                let (items, create) = (self.items.clone(), self.create.clone());
                let connectable = parameter_rows::connectable(&self.target, &self.row);
                Button::new(SharedString::from(format!("{}-connect", self.form)))
                    .icon(gpui_kit::assets::IconName::Network)
                    .xsmall()
                    .tooltip(if connectable {
                        "Connect a texture or Spectrum Data"
                    } else {
                        "Connect Spectrum Data"
                    })
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = menu.scrollable(true).max_h(px(360.));
                        let entries = parameter_rows::connect_menu(&target, &row, &items, &create);
                        // Two adjacent non-empty sections get a separator.
                        let count = entries.sections().count();
                        for (index, section) in entries.sections().enumerate() {
                            if index > 0 && index < count {
                                menu = menu.separator();
                            }
                            for (label, edits) in section {
                                let this = this.clone();
                                let edits = edits.clone();
                                menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                                    move |_, _, cx| {
                                        let _ = this.update(cx, |this, cx| {
                                            this.send_all(edits.clone(), cx)
                                        });
                                    },
                                ));
                            }
                        }
                        menu
                    })
            });
        // An imagemap's filename picks one of the project's image assets; a
        // texture input connects one directly (core reuses or creates the
        // imagemap of the row's lineage).
        let file_images = parameter_rows::image_choices(&self.target, &self.row, &self.items);
        let (images, edit_for): (_, fn(&Value, &parameter_rows::Row, &str) -> Value) =
            if file_images.is_empty() {
                (
                    parameter_rows::connect_image_choices(&self.target, &self.row, &self.items),
                    parameter_rows::connect_image_edit,
                )
            } else {
                (file_images, parameter_rows::image_edit)
            };
        let choose_image = (!images.is_empty()).then(|| {
            let this = cx.entity().downgrade();
            let (target, row) = (self.target.clone(), self.row.clone());
            Button::new(SharedString::from(format!("{}-image", self.form)))
                .icon(gpui_kit::assets::IconName::Image)
                .xsmall()
                .tooltip("Choose an image")
                .dropdown_menu(move |menu, _, _| {
                    let mut menu = menu.scrollable(true).max_h(px(360.));
                    for (name, id) in &images {
                        let edit = edit_for(&target, &row, id);
                        let this = this.clone();
                        menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                            move |_, _, cx| {
                                let edit = edit.clone();
                                let _ = this.update(cx, |this, cx| this.send(edit, cx));
                            },
                        ));
                    }
                    menu
                })
        });
        let feedback = parameter_form::feedback_text(&self.message, &self.session, &self.form, cx);
        let label = if self.row.declared {
            rgb(0x9098a6)
        } else {
            rgb(0x676d78)
        };
        let value = self.value(cx);
        div()
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Undo, window, cx| {
                    if !this.draft_history
                        && !this.is_modified(cx)
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
                        && !this.is_modified(cx)
                        && !this.session.read(cx).edit_feedback.is_pending(&this.form)
                    {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(super::menu::RedoEdit), cx);
                    }
                },
            ))
            .flex()
            .flex_col()
            .py(px(2.))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .w(px(92.))
                            .flex_shrink_0()
                            .truncate()
                            .text_color(label)
                            .child(self.row.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            // Greyed until declared, and while the
                            // conductor's reflectance reads in eta/k's
                            // place.
                            .when(!self.row.declared || self.row.ignored.is_some(), |this| {
                                this.opacity(0.55)
                            })
                            .child(value),
                    )
                    .when_some(choose_image, |this, button| this.child(button))
                    .when_some(connect, |this, button| this.child(button))
                    .when_some(reset, |this, edit| {
                        this.child(
                            Button::new(SharedString::from(format!("{}-reset", self.form)))
                                .icon(if connected {
                                    gpui_kit::assets::IconName::Minus
                                } else {
                                    gpui_kit::assets::IconName::Undo2
                                })
                                .xsmall()
                                .tooltip(if connected {
                                    "Disconnect"
                                } else {
                                    "Reset to default"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.send_all(edit.clone(), cx)
                                })),
                        )
                    }),
            )
            .when(!self.row.declared && !self.row.hint.is_empty(), |this| {
                this.child(
                    div()
                        .pl(px(100.))
                        .text_xs()
                        .text_color(rgb(0x676d78))
                        .child(self.row.hint.clone()),
                )
            })
            // The conductor's unused eta/k rows carry their hint
            // even declared — amber, the warning colour.
            .when_some(self.row.ignored, |this, hint| {
                this.child(
                    div()
                        .pl(px(100.))
                        .text_xs()
                        .text_color(rgb(0xd0a86a))
                        .child(hint),
                )
            })
            .when(!self.row.known && self.row.declared, |this| {
                this.child(
                    div()
                        .pl(px(100.))
                        .text_xs()
                        .text_color(rgb(0xd0a86a))
                        .child("not in this implementation's parameter table"),
                )
            })
            .when(self.is_modified(cx) && feedback.is_empty(), |this| {
                this.child(
                    div()
                        .pl(px(100.))
                        .text_xs()
                        .text_color(rgb(0x9098a6))
                        .child("Enter to apply"),
                )
            })
            .when(!feedback.is_empty(), |this| {
                this.child(
                    div()
                        .pl(px(100.))
                        .text_xs()
                        .text_color(rgb(0xd99a74))
                        .child(feedback),
                )
            })
    }
}

/// Read-only rows (the Viewer): the same typed rows as text, undeclared
/// table rows greyed with their default or hint. `keep` filters by name.
/// The read-only page rows, each with the parameter name it renders
/// under, so an Inspector page can weave an extra element between
/// them.
pub fn read_only_rows(page: &Value, keep: &dyn Fn(&str) -> bool) -> Vec<(String, AnyElement)> {
    parameter_rows::rows(page)
        .into_iter()
        .filter(|row| keep(&row.name))
        .map(|row| {
            let name = row.name.clone();
            let text = if row.declared {
                let truncated = if row.parameter["preview_truncated"] == true {
                    " … (read-only preview)"
                } else {
                    ""
                };
                format!(
                    "{}{truncated}",
                    super::components::values_text(&row.parameter["values"])
                )
            } else {
                match &row.field {
                    Field::Number { value, .. } if !value.is_empty() => value.clone(),
                    Field::Rgb(rgb) | Field::Vector(rgb) if row.hint.is_empty() => {
                        format!("{} {} {}", rgb[0], rgb[1], rgb[2])
                    }
                    Field::Spectrum(name) | Field::Text(name) if !name.is_empty() => name.clone(),
                    Field::Bool(value) if row.hint.is_empty() => value.to_string(),
                    _ => format!("({})", row.hint),
                }
            };
            let color = if row.declared {
                rgb(0xc4c8d0)
            } else {
                rgb(0x676d78)
            };
            let row_element = div()
                .flex()
                .gap_2()
                .py(px(2.))
                .child(
                    div()
                        .w(px(92.))
                        .flex_shrink_0()
                        .text_color(rgb(0x9098a6))
                        .child(row.name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(color)
                        .child(text),
                )
                .into_any_element();
            (name, row_element)
        })
        .collect()
}

/// The read-only Metal colour row of a conductor page — the
/// wire's conductor-role answer (the F0 of eta/k, a missing side filled
/// with r4's copper, or the reflectance's own colour; whichever metal row
/// asked, the answer is the same), with the `source` spelled out so the
/// grey it answers for r4's refused reflectance-beside-eta/k pair (and a
/// texture connection's "not evaluated") stays readable without a hover.
/// Not a parameter row: nothing edits through it; `parameter` only names
/// the ask (the page's eta or conductor.eta). The row truncates; the
/// full source — both sides, their names and resources — rides the
/// hover.
pub fn metal_colour_row(
    session: &Entity<Session>,
    target: &Value,
    parameter: &str,
    cx: &mut App,
) -> impl IntoElement {
    let answer = session.update(cx, |session, cx| {
        session.parameter_swatch(target, parameter, Some("conductor"), cx)
    });
    // item 13: no resolved colour paints the hollow look — the
    // filled muted dark read as "0". While the answer is on its way or
    // was refused the box stays hollow with the reason on hover.
    let mut paint = None;
    let mut text = "resolving the Metal colour…".to_owned();
    if let Some(answer) = &answer {
        let channel = |index: usize| {
            answer["rgb"]
                .as_array()
                .and_then(|values| values.get(index))
                .and_then(Value::as_f64)
                .unwrap_or_default()
                .clamp(0., 1.)
        };
        if let Some(error) = answer.get("error").and_then(Value::as_str) {
            text = error.to_owned();
        } else {
            paint = Some(Rgba {
                r: channel(0) as f32,
                g: channel(1) as f32,
                b: channel(2) as f32,
                a: 1.,
            });
            text = answer["source"].as_str().unwrap_or_default().to_owned();
        }
    }
    let mut box_div = div().w(px(18.)).h(px(16.)).flex_shrink_0().rounded_sm();
    box_div = if let Some(paint) = paint {
        box_div.bg(paint)
    } else {
        box_div
            .overflow_hidden()
            .child(super::components::unset_swatch())
    };
    div()
        .flex()
        .gap_2()
        .py(px(2.))
        .child(
            div()
                .w(px(92.))
                .flex_shrink_0()
                .truncate()
                .text_color(rgb(0x9098a6))
                .child("Metal colour"),
        )
        .child(box_div)
        .child(
            div()
                .id(SharedString::from(format!(
                    "metal-colour-source-{}",
                    target["id"].as_str().unwrap_or_default()
                )))
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(rgb(0x9098a6))
                .truncate()
                // The row truncates; the full source —
                // both sides' resources and names — rides the hover.
                .tooltip(super::components::text_tooltip(text.clone()))
                .child(text),
        )
}

/// The Type selector of one component or graph node. A choice asks the
/// core for a plan, so the window can confirm
/// removed or unknown parameters before applying. None in the Viewer and
/// for kinds without candidates.
pub fn implementation_selector(
    session: &Entity<Session>,
    component: &Value,
    target: &Value,
    schema: &Value,
    read_only: bool,
    feedback: &super::edit_feedback::EditFeedback,
) -> Option<AnyElement> {
    let choices = super::implementation_menu::choices(component, schema);
    if read_only || choices.is_empty() {
        return None;
    }
    let current = component["implementation"].as_str()?.to_owned();
    let form = format!(
        "implementation-{}",
        target["id"].as_str().unwrap_or_default()
    );
    let message = feedback.message(&form).map(str::to_owned);
    let session = session.clone();
    let target = target.clone();
    let button = Button::new(SharedString::from(form.clone()))
        .label(current)
        .xsmall()
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu;
            for name in &choices {
                let session = session.clone();
                let target = target.clone();
                let form = form.clone();
                let name = name.clone();
                menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                    session.update(cx, |session, cx| {
                        let project = session.project_value();
                        if project.is_object() && !session.document().is_read_only() {
                            session.edit_feedback.submitted(&form);
                            session.send(
                                Command::PlanImplementation {
                                    project,
                                    target: target.clone(),
                                    implementation: name.clone(),
                                    form: form.clone(),
                                },
                                cx,
                            );
                        }
                    })
                }));
            }
            menu
        });
    Some(
        div()
            .flex()
            .flex_col()
            .child(button)
            .when_some(message, |this, message| {
                this.child(div().text_color(rgb(0xd99a74)).child(message))
            })
            .into_any_element(),
    )
}

/// Shared by Inspector and Node Editor; core owns graph compatibility checks.
pub fn texture_value_type_selector(
    session: &Entity<Session>,
    component: &Value,
    target: &Value,
    read_only: bool,
    feedback: &super::edit_feedback::EditFeedback,
) -> Option<AnyElement> {
    if read_only || component["kind"] != "texture" {
        return None;
    }
    let current = component["texture_value_type"].as_str()?.to_owned();
    let id = target["id"].as_str()?.to_owned();
    let form = format!("texture-value-type-{id}");
    let message = feedback.message(&form).map(str::to_owned);
    let session = session.clone();
    let button = Button::new(SharedString::from(form.clone())).label(format!("Value: {current}"))
        .xsmall().dropdown_menu(move |menu, _, _| {
            let mut menu = menu;
            for value in ["float", "spectrum"] {
                let session = session.clone();
                let id = id.clone();
                let form = form.clone();
                menu = menu.item(PopupMenuItem::new(value).on_click(move |_, _, cx| {
                    session.update(cx, |session, cx| {
                        let project = session.project_value();
                        if project.is_object() && !session.document().is_read_only() {
                            session.edit_feedback.submitted(&form);
                            session.send(Command::Apply { project, edits: vec![serde_json::json!({"kind":"set_texture_value_type","resource_id":id,"value":value})], follow: Follow::Nothing, form: Some(form.clone()) }, cx);
                        }
                    });
                }));
            }
            menu
        });
    Some(
        div()
            .flex()
            .flex_col()
            .child(button)
            .when_some(message, |this, message| {
                this.child(div().text_color(rgb(0xd99a74)).child(message))
            })
            .into_any_element(),
    )
}

#[cfg(test)]
mod integer_step_tests {
    use super::stepped_integer;
    use gpui_kit::component::input::StepAction;

    #[test]
    fn stepping_observes_bounds_and_saturates() {
        assert_eq!(stepped_integer(1, StepAction::Decrement, Some(1)), 1);
        assert_eq!(stepped_integer(0, StepAction::Decrement, None), -1);
        assert_eq!(
            stepped_integer(i64::MAX, StepAction::Increment, None),
            i64::MAX
        );
    }

    #[test]
    fn a_run_of_steps_produces_one_commit_signal() {
        let actions = [
            StepAction::Increment,
            StepAction::Increment,
            StepAction::Decrement,
            StepAction::Increment,
        ];
        let final_value = actions
            .into_iter()
            .fold(8, |value, action| stepped_integer(value, action, Some(1)));
        assert_eq!(final_value, 10);
        let mut baseline = Some(8);
        assert!(super::take_step_commit(&mut baseline, Some(final_value)));
        assert!(!super::take_step_commit(&mut baseline, Some(final_value)));
    }
}
