//! The colour dialog of a spectrum-capable parameter row is one popover
//! over the row's colour box with the
//! current approximate colour and value, the tabs the schema allows —
//! rgb / blackbody / spectrum, each gated on the table's value types —
//! and, for a light, its `scale` beside the blackbody temperature. The
//! rgb tab edits its value as colour × intensity: the wheel
//! picks the colour part,
//! the number fields keep the real, unnormalised triple and the
//! Intensity field scales the colour for the illuminant rows. Every
//! change leaves through one `project.apply` transaction (one Undo
//! step); switching tab replaces the value with the tab type's default.
use super::Session;
use super::colour_wheel::{ColourWheel, ColourWheelEvent, slider_value};
use super::parameter_rows::{self, Row, SpectrumTab};
use crate::backend::{Command, Follow};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{
    Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use std::rc::Rc;

/// The number a dialog field shows: whole values without a trailing
/// ".0" (the same spelling the spectrum editor's cells keep).
fn field_number(value: f64) -> String {
    if value == value.trunc() && value.is_finite() && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

/// (items 1 and 3): whether a list row's `label` matches the
/// search field's `query` — a case-insensitive substring, an empty or
/// blank query matching everything.
fn matches_search(query: &str, label: &str) -> bool {
    label
        .to_lowercase()
        .contains(query.trim().to_lowercase().as_str())
}

/// The colour dialogs currently open, by their
/// parameter form id. A tab click rewrites the value's type, the
/// Inspector rebuilds the row's field, and the popover's open state —
/// gpui element state keyed under the field's own entity — dies with
/// it, so a rebuilt field would open with its dialog closed again (a
/// blackbody drag keeps the type and the entity, which is why its
/// dialog survives). The popover reports every announced open and
/// close into this bookkeeping on the session, which outlives the
/// rebuild; a field whose form is marked open renders its popover
/// open (the brief's minimal approach), and the flag clears when the
/// user dismisses the dialog.
#[derive(Default)]
pub(crate) struct OpenDialogs {
    open: std::collections::HashSet<String>,
}

impl OpenDialogs {
    /// Whether the field of `form` renders its popover open.
    pub(crate) fn is_open(&self, form: &str) -> bool {
        self.open.contains(form)
    }

    /// The popover's own report: the user opened the dialog, or
    /// dismissed it (outside click, Escape, the trigger again).
    pub(crate) fn set_open(&mut self, form: &str, open: bool) {
        if open {
            self.open.insert(form.to_owned());
        } else {
            self.open.remove(form);
        }
    }
}

/// The blackbody slider's range, in kelvin: the same span the crate's
/// Planck integration clamps to.
const KELVIN_RANGE: (f64, f64) = (1000., 40000.);

/// The slider position a kelvin maps to — logarithmic
/// over [`KELVIN_RANGE`], so a given drag changes the temperature by
/// the same fraction everywhere (the linear slider took 5500 K down to
/// 1400 K in 30 px).
pub(super) fn kelvin_fraction(kelvin: f64) -> f32 {
    let (min, max) = KELVIN_RANGE;
    let kelvin = kelvin.clamp(min, max);
    ((kelvin.ln() - min.ln()) / (max.ln() - min.ln())) as f32
}

/// The kelvin a slider position names — the
/// logarithmic mapping's other direction (`fraction` clamped to the
/// track).
pub(super) fn fraction_kelvin(fraction: f64) -> f64 {
    let (min, max) = KELVIN_RANGE;
    (fraction.clamp(0., 1.) * (max.ln() - min.ln()) + min.ln()).exp()
}

/// The temperature slider: a 0..1 track over the logarithmic mapping
/// Each 0.001 step scales the kelvin by 40^0.001 — about
/// 0.37% (20 K at 5500 K), finer than the drag itself.
fn kelvin_slider_state(kelvin: f64) -> SliderState {
    SliderState::new()
        .max(1.)
        .min(0.)
        .step(0.001)
        .default_value(kelvin_fraction(kelvin))
}

/// Which number fields an Enter applies.
#[derive(Clone, Copy)]
enum FieldGroup {
    Rgb,
    Kelvin,
    Scale,
    Intensity,
    /// The scalar of a float-declared spectrum slot (dielectric's eta),
    /// the fourth tab's own number.
    Number,
}

/// The rgb triple a declared rgb row carries, else white (the rgb tab's
/// default once its fields are first touched).
fn row_rgb(row: &Row) -> [f64; 3] {
    let channel = |index: usize| {
        row.parameter["values"]["value"]
            .as_array()
            .and_then(|values| values.get(index))
            .and_then(|value| value.as_f64().or_else(|| value["value"].as_f64()))
            .unwrap_or(1.)
    };
    [channel(0), channel(1), channel(2)]
}

/// The temperature a declared blackbody row carries, else daylight
/// (6500 K, the tab switch's default).
fn row_kelvin(row: &Row) -> f64 {
    row.parameter["values"]["value"][0]
        .as_f64()
        .or_else(|| row.parameter["values"]["value"][0]["value"].as_f64())
        .unwrap_or(6500.)
}

/// A scale row's number text (its greyed default included).
fn scale_number(row: &Row) -> String {
    match &row.field {
        parameter_rows::Field::Number { value, .. } => value.clone(),
        _ => "1".into(),
    }
}

/// The number a float-declared spectrum slot carries (dielectric's
/// eta): the text its scalar field shows — the greyed default too, so
/// the page edits an undeclared slot the same way.
fn row_number(row: &Row) -> String {
    match &row.field {
        parameter_rows::Field::Number { value, .. } => value.clone(),
        _ => String::new(),
    }
}

/// Whether the page shows the row's scalar number field: a float value
/// on a spectrum-capable slot, declared or greyed — the shapes the
/// other tabs give editors of their own (an rgb triple, a blackbody
/// temperature, a sample table) do not take it.
fn number_shown(row: &Row) -> bool {
    matches!(&row.field, parameter_rows::Field::Number { .. }) && row.value_type == "float"
}

/// The scalar's edit — the plain float `set_parameter` the row's own
/// number field would write, declaring the value under the row's name
/// so the owner parameter keeps its type.
fn number_edits(target: &Value, row: &Row, value: f64) -> Vec<Value> {
    vec![parameter_rows::set_edit(
        target,
        row,
        json!({"kind":"floats","value":[value]}),
    )]
}

/// The project's SpectrumData resources as menu rows: (label, id),
/// sorted by name — the order every Contents choice lists.
fn spectrum_data_choices(items: &Value) -> Vec<(String, String)> {
    let mut choices = items
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, item)| item["resource_kind"] == "spectrum_data")
        .map(|(id, item)| (item["name"].as_str().unwrap_or(id).to_owned(), id.clone()))
        .collect::<Vec<_>>();
    choices.sort();
    choices
}

/// The pickers' row height budget: about 200 px of list inside the
/// dialog, the rest scrolls.
const LIST_MAX_HEIGHT: Pixels = px(200.);

/// One clickable row of an in-dialog picker. The
/// choices are inside the popover on purpose — a menu outside it dies
/// with the popover's own mouse-down-out dismissal before its item's
/// click can ever land, so the dialog lists them itself. `current`
/// highlights the choice the row names now; a click applies the
/// `spectrum`-typed `values` through the dialog's one transaction.
fn choice_row(
    id: SharedString,
    label: String,
    current: bool,
    this: &WeakEntity<ColourEdit>,
    values: Value,
) -> Stateful<Div> {
    let this = this.clone();
    div()
        .id(id)
        .px(px(6.))
        .py(px(2.))
        .rounded_sm()
        .cursor_pointer()
        .truncate()
        .text_color(if current {
            rgb(0xe6e8ee)
        } else {
            rgb(0xc4c8d0)
        })
        .when(current, |row| row.bg(rgb(0x2f343d)))
        .hover(|style| style.bg(rgb(0x262a31)))
        .child(label)
        .on_click(move |_, _, cx| {
            let _ = this.update(cx, |this, cx| {
                let edits = parameter_rows::colour_edits(
                    &this.target,
                    &this.row,
                    "spectrum",
                    values.clone(),
                );
                this.send_edits(edits, cx);
            });
        })
}

/// The scrollable body an in-dialog picker's rows sit
/// in — at most [`LIST_MAX_HEIGHT`] tall, so the dialog stays on the
/// row it belongs to.
fn choice_list(id: SharedString, rows: Vec<Stateful<Div>>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .max_h(LIST_MAX_HEIGHT)
        .overflow_y_scroll()
        .children(rows)
}

/// The text a header shows for the row's current
/// value, the way a person reads it — a Spectrum Data reference names its
/// resource, a named spectrum its name without the wire's quotes, a
/// blackbody its temperature, an rgb its channels, a constant float its
/// number. An unset row keeps its hint; shapes without a spelling of
/// their own (inline pairs) keep the values text.
fn value_text(row: &Row, items: &Value) -> String {
    if !row.declared {
        return format!("({})", row.hint);
    }
    match &row.field {
        parameter_rows::Field::Rgb(channels) => format!(
            "RGB {:.2}, {:.2}, {:.2}",
            channels[0], channels[1], channels[2]
        ),
        parameter_rows::Field::Number { value, .. } if row.value_type == "blackbody" => {
            format!("Blackbody {value} K")
        }
        parameter_rows::Field::Number { value, .. } => value.clone(),
        parameter_rows::Field::Spectrum(name) => format!("{name} · Named"),
        parameter_rows::Field::SpectrumData(id) => format!(
            "{} · Spectrum Data",
            super::components::item_name(items, id)
        ),
        _ => super::components::values_text(&row.parameter["values"]),
    }
}

/// The RGB tab stores the value as
/// colour × intensity so a light's brightness can top 1. A value
/// whose biggest channel tops 1 splits into colour = value / max and
/// intensity = max; anything else carries itself at intensity 1 (the
/// number fields keep the exact values, the wheel shows the colour).
fn split_intensity(rgb: [f64; 3]) -> ([f64; 3], f64) {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    if max > 1. {
        (rgb.map(|channel| channel / max), max)
    } else {
        (rgb, 1.)
    }
}

/// colour × intensity: the value every RGB-tab apply leaves as.
fn merge_intensity(colour: [f64; 3], intensity: f64) -> [f64; 3] {
    colour.map(|channel| channel * intensity)
}

/// Whether the row plays the illuminant role the intensity multiplier
/// is for: a texture input of the schema's `spectrum_illuminant` kind,
/// or one of the light colour rows — the tables whose value types allow
/// blackbody are only the lights' I and L (the same signal the blackbody
/// tab's scale ride-along keys on).
fn illuminant_row(row: &Row) -> bool {
    row.texture_kind.as_deref() == Some("spectrum_illuminant")
        || parameter_rows::spectrum_tabs(row).blackbody
}

/// Whether the RGB tab shows the Intensity field: an illuminant-role
/// row, or a current value whose biggest channel is over 1 —
/// reflectance-like rows stay 0..1 without it.
fn intensity_shown(row: &Row) -> bool {
    illuminant_row(row) || row_rgb(row).into_iter().fold(f64::MIN, f64::max) > 1.
}

/// The one undoable rgb edit every RGB-tab apply leaves — the typed
/// channels with their explicit colour space kept on an rgb-to-rgb
/// carry (see [`parameter_rows::colour_edits`]).
fn rgb_edits(target: &Value, row: &Row, rgb: [f64; 3]) -> Vec<Value> {
    parameter_rows::colour_edits(
        target,
        row,
        "rgb",
        json!({"kind":"floats","value":[rgb[0], rgb[1], rgb[2]]}),
    )
}

/// 's colour editor, shared by the two surfaces that edit a
/// spectrum-capable parameter: the popover behind a row's colour box
/// or curve sample (`ParameterField`), and the Inspector's inline
/// value-node page. All the state and the applies live here.
pub(super) struct ColourEdit {
    session: Entity<Session>,
    target: Value,
    project: Value,
    row: Row,
    /// The light's `scale`, shown beside the blackbody temperature when
    /// the page allows one.
    scale: Option<Row>,
    items: Rc<Value>,
    form: String,
    /// The rgb fields' drafts (three), the kelvin number, the scale
    /// number and the intensity number, in that order — Enter in any
    /// of them applies its edit.
    rgb: Vec<Entity<InputState>>,
    kelvin: Entity<InputState>,
    kelvin_slider: Entity<SliderState>,
    scale_input: Option<Entity<InputState>>,
    /// The colour × intensity multiplier's draft, shown for the
    /// illuminant rows (and any value wider than 1) — built always,
    /// rendered where it has a role.
    intensity_input: Entity<InputState>,
    /// The scalar draft of a float-declared spectrum slot (dielectric's
    /// eta): built always, rendered on the Data tab the row opens on.
    number_input: Entity<InputState>,
    /// The colour part (0..1) and the intensity the confirmed value
    /// splits into — the carry every RGB-tab apply leaves through,
    /// refreshed with the row.
    colour_value: [f64; 3],
    intensity_value: f64,
    /// The hue / saturation disc with its value slider, the wheel the
    /// RGB tab edits the colour part with.
    wheel: Entity<ColourWheel>,
    /// The Named tab's search — the case-insensitive
    /// substring its list filters by.
    named_search: Entity<InputState>,
    /// Whether the Named tab lists every built-in
    /// name, beyond the row's own family.
    named_all: bool,
    /// The Spectrum Data tab's search, the same
    /// filter its list reads.
    data_search: Entity<InputState>,
    /// What the drafts were built from: a refresh adopts a field only
    /// when the value behind it changed, so our own applies' echo never
    /// fights the typing or a drag in progress.
    built: Vec<String>,
    message: String,
}

impl ColourEdit {
    /// Create one dialog behind a spectrum-capable row. `page` is the
    /// field's own page, which decides whether a light's `scale` rides
    /// along. (gpui constructors take the window and the context by
    /// necessity; the construction inputs are the six before them.)
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
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
        let scale = parameter_rows::dialog_scale(&parameter_rows::rows(page));
        let rgb_values = row_rgb(&row);
        let (colour_value, intensity_value) = split_intensity(rgb_values);
        let kelvin = row_kelvin(&row);
        let kelvin_text = field_number(kelvin);
        let scale_text = scale.as_ref().map(scale_number).unwrap_or_default();
        let intensity_text = field_number(intensity_value);
        let number_text = row_number(&row);
        let mut rgb = Vec::new();
        for channel in rgb_values {
            let input =
                cx.new(|cx| InputState::new(window, cx).default_value(field_number(channel)));
            Self::wire_input(&input, FieldGroup::Rgb, cx);
            rgb.push(input);
        }
        let kelvin_input =
            cx.new(|cx| InputState::new(window, cx).default_value(kelvin_text.clone()));
        Self::wire_input(&kelvin_input, FieldGroup::Kelvin, cx);
        let kelvin_slider = cx.new(|_| kelvin_slider_state(kelvin));
        cx.subscribe_in(
            &kelvin_slider,
            window,
            |this, _, event: &SliderEvent, window, cx| {
                // The track is a position in the log mapping;
                // the temperature it names is the whole kelvin the number
                // field spells.
                let kelvin = fraction_kelvin(slider_value(event)).round();
                match event {
                    // The drag previews in the number field only; the apply
                    // rides the release, so one drag is one Undo step.
                    SliderEvent::Change(_) => {
                        let text = field_number(kelvin);
                        let kelvin = this.kelvin.clone();
                        kelvin.update(cx, |input, cx| input.set_value(text, window, cx));
                    }
                    SliderEvent::Release(_) => this.apply_kelvin(kelvin, cx),
                }
                cx.notify();
            },
        )
        .detach();
        let scale_input = scale.clone().map(|row| {
            let text = scale_number(&row);
            let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
            Self::wire_input(&input, FieldGroup::Scale, cx);
            input
        });
        let intensity_input =
            cx.new(|cx| InputState::new(window, cx).default_value(intensity_text.clone()));
        Self::wire_input(&intensity_input, FieldGroup::Intensity, cx);
        let number_input =
            cx.new(|cx| InputState::new(window, cx).default_value(number_text.clone()));
        Self::wire_input(&number_input, FieldGroup::Number, cx);
        let wheel = cx.new(|cx| ColourWheel::new(colour_value, window, cx));
        cx.subscribe_in(
            &wheel,
            window,
            |this, _, event: &ColourWheelEvent, window, cx| match event {
                // The drag previews in the number fields only; the
                // apply rides the release, so one drag is one Undo step.
                ColourWheelEvent::Change(colour) => this.preview_colour(*colour, window, cx),
                ColourWheelEvent::Release(colour) => this.apply_colour(*colour, cx),
            },
        )
        .detach();
        // These are the searches of the two in-dialog lists. Enter
        // applies nothing; every keystroke re-filters the
        // list it belongs to.
        let named_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search named spectra"));
        cx.observe(&named_search, |_, _, cx| cx.notify()).detach();
        let data_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search Spectrum Data"));
        cx.observe(&data_search, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            target,
            project,
            row,
            scale,
            items,
            form,
            rgb,
            kelvin: kelvin_input,
            kelvin_slider,
            scale_input,
            intensity_input,
            number_input,
            colour_value,
            intensity_value,
            wheel,
            named_search,
            named_all: false,
            data_search,
            built: rgb_values
                .map(field_number)
                .into_iter()
                .chain([kelvin_text, scale_text, intensity_text, number_text])
                .collect(),
            message: String::new(),
        }
    }

    /// Enter applies in the number fields, like every typed field here —
    /// each field group applies its own edit.
    fn wire_input(input: &Entity<InputState>, group: FieldGroup, cx: &mut Context<Self>) {
        cx.subscribe(input, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.apply_entered(group, cx);
            }
        })
        .detach();
        cx.observe(input, |_, _, cx| cx.notify()).detach();
    }

    /// Which field group an Enter applies.
    fn apply_entered(&mut self, group: FieldGroup, cx: &mut Context<Self>) {
        let parse = |text: &str| text.trim().parse::<f64>().ok().filter(|n| n.is_finite());
        let number = |text: String| {
            let text = text.trim().to_owned();
            if text.is_empty() { None } else { parse(&text) }
        };
        match group {
            FieldGroup::Rgb => {
                let texts = self
                    .rgb
                    .iter()
                    .map(|input| input.read(cx).value().to_string())
                    .collect::<Vec<_>>();
                if texts.iter().all(|text| text.trim().is_empty()) {
                    return;
                }
                match texts
                    .iter()
                    .map(|text| parse(text))
                    .collect::<Option<Vec<f64>>>()
                {
                    Some(rgb) if rgb.len() == 3 => self.apply_rgb([rgb[0], rgb[1], rgb[2]], cx),
                    _ => {
                        self.message = "Enter three numbers".into();
                        cx.notify();
                    }
                }
            }
            FieldGroup::Kelvin => {
                if let Some(kelvin) = number(self.kelvin.read(cx).value().to_string()) {
                    self.apply_kelvin(kelvin, cx);
                } else if !self.kelvin.read(cx).value().trim().is_empty() {
                    self.message = "Enter a temperature".into();
                    cx.notify();
                }
            }
            FieldGroup::Scale => {
                let Some(input) = self.scale_input.clone() else {
                    return;
                };
                match number(input.read(cx).value().to_string()) {
                    Some(value) => self.apply_scale(value, cx),
                    None if !input.read(cx).value().trim().is_empty() => {
                        self.message = "Enter a number".into();
                        cx.notify();
                    }
                    None => {}
                }
            }
            FieldGroup::Intensity => {
                let input = self.intensity_input.clone();
                match number(input.read(cx).value().to_string()) {
                    Some(value) if value >= 0. => self.apply_intensity(value, cx),
                    Some(_) => {
                        self.message = "Intensity cannot be negative".into();
                        cx.notify();
                    }
                    None if !input.read(cx).value().trim().is_empty() => {
                        self.message = "Enter a number".into();
                        cx.notify();
                    }
                    None => {}
                }
            }
            FieldGroup::Number => {
                let input = self.number_input.clone();
                match number(input.read(cx).value().to_string()) {
                    Some(value) => self.apply_number(value, cx),
                    None if !input.read(cx).value().trim().is_empty() => {
                        self.message = "Enter a number".into();
                        cx.notify();
                    }
                    None => {}
                }
            }
        }
    }

    /// The parent field's confirmed refresh: adopt a field only where
    /// the value behind it changed, so typing and drags survive our own
    /// applies' echo. (The `scale` snapshot rides along unchanged from
    /// construction: the popover is short-lived and its own message
    /// line reports every apply's outcome.)
    pub(super) fn refresh(
        &mut self,
        project: &Value,
        row: &Row,
        scale: Option<Row>,
        items: Rc<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project = project.clone();
        self.row = row.clone();
        self.scale = scale;
        self.items = items;
        let rgb_values = row_rgb(&self.row);
        let (colour, intensity) = split_intensity(rgb_values);
        self.colour_value = colour;
        self.intensity_value = intensity;
        let rgb_texts = rgb_values.map(field_number);
        let kelvin_text = field_number(row_kelvin(&self.row));
        let scale_text_now = self.scale.as_ref().map(scale_number).unwrap_or_default();
        let intensity_text = field_number(intensity);
        let number_text = row_number(&self.row);
        let previous = std::mem::replace(
            &mut self.built,
            rgb_texts
                .iter()
                .cloned()
                .chain([
                    kelvin_text.clone(),
                    scale_text_now.clone(),
                    intensity_text.clone(),
                    number_text.clone(),
                ])
                .collect(),
        );
        let changed = |index: usize, text: &str| previous.get(index).is_none_or(|was| was != text);
        // The rgb fields and the wheel follow a changed rgb value only —
        // the wheel over the colour part, the divider the fields keep.
        if (0..3).any(|index| changed(index, &rgb_texts[index])) {
            for (input, text) in self.rgb.iter().zip(rgb_texts.iter()) {
                let (input, text) = (input.clone(), text.clone());
                if input.read(cx).value() != text.as_str() {
                    input.update(cx, |input, cx| input.set_value(text, window, cx));
                }
            }
            let wheel = self.wheel.clone();
            wheel.update(cx, |wheel, cx| wheel.set_colour(colour, window, cx));
        }
        // The kelvin slider and its number follow a changed temperature.
        if changed(3, &kelvin_text) {
            self.kelvin_slider.update(cx, |slider, cx| {
                slider.set_value(kelvin_fraction(row_kelvin(&self.row)), window, cx)
            });
            if self.kelvin.read(cx).value() != kelvin_text.as_str() {
                let text = kelvin_text.clone();
                let input = self.kelvin.clone();
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            }
        }
        // The light's scale follows its confirmed value too (an Inspector
        // edit, Undo or another client), so Enter never writes a stale one.
        if changed(4, &scale_text_now)
            && let Some(input) = self.scale_input.clone()
            && input.read(cx).value() != scale_text_now.as_str()
        {
            let text = scale_text_now.clone();
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        // The intensity follows the confirmed value's own split too (an
        // Inspector edit, Undo or another client), so Enter never
        // writes a stale multiplier.
        if changed(5, &intensity_text)
            && self.intensity_input.read(cx).value() != intensity_text.as_str()
        {
            let input = self.intensity_input.clone();
            input.update(cx, |input, cx| input.set_value(intensity_text, window, cx));
        }
        // The scalar follows its confirmed value the same way — an
        // Undo or another client's edit rewrites eta, Enter never
        // writes the stale one back.
        if changed(6, &number_text) && self.number_input.read(cx).value() != number_text.as_str() {
            let input = self.number_input.clone();
            input.update(cx, |input, cx| input.set_value(number_text, window, cx));
        }
        self.message.clear();
        cx.notify();
    }

    /// The edits that leave from here — one transaction, one Undo step —
    /// the field's own form id routing the apply feedback.
    fn send_edits(&mut self, edits: Vec<Value>, cx: &mut Context<Self>) {
        let project = self.session.read(cx).project_value();
        if project != self.project || !project.is_object() {
            self.message =
                "Project changed; wait for the Inspector to refresh before applying".into();
            cx.notify();
            return;
        }
        if self.session.read(cx).document().is_read_only() {
            self.message = "Viewer is read-only — Save as Project to edit".into();
            cx.notify();
            return;
        }
        let form = self.form.clone();
        self.session.update(cx, |session, cx| {
            session.edit_feedback.submitted(&form);
            session.send(
                Command::Apply {
                    project,
                    edits,
                    follow: Follow::Nothing,
                    form: Some(form),
                },
                cx,
            )
        });
        cx.notify();
    }

    /// The typed rgb triple: the fields carry the real, unnormalised
    /// value exactly as entered — nothing is lost, and the confirmed
    /// refresh re-splits it.
    fn apply_rgb(&mut self, rgb: [f64; 3], cx: &mut Context<Self>) {
        let edits = rgb_edits(&self.target, &self.row, rgb);
        self.send_edits(edits, cx);
    }

    /// The wheel or its value slider: the colour it reached, carried
    /// rgb = colour × intensity, one undoable edit.
    fn apply_colour(&mut self, colour: [f64; 3], cx: &mut Context<Self>) {
        self.apply_rgb(merge_intensity(colour, self.intensity_value), cx);
    }

    /// The intensity multiplier: the confirmed colour carried at the
    /// typed intensity, the same one undoable edit. Zero yields the
    /// colour's black; negative ones would flip the colour, not dim
    /// the light.
    fn apply_intensity(&mut self, intensity: f64, cx: &mut Context<Self>) {
        self.apply_rgb(merge_intensity(self.colour_value, intensity), cx);
    }

    /// A wheel gesture's preview: the rgb fields follow the colour
    /// reached (at the confirmed intensity), the apply still riding
    /// the release.
    fn preview_colour(&mut self, colour: [f64; 3], window: &mut Window, cx: &mut Context<Self>) {
        let carried = merge_intensity(colour, self.intensity_value);
        for (input, channel) in self.rgb.iter().zip(carried) {
            let text = field_number(channel);
            if input.read(cx).value() != text.as_str() {
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            }
        }
    }

    fn apply_kelvin(&mut self, kelvin: f64, cx: &mut Context<Self>) {
        let kelvin = kelvin.clamp(KELVIN_RANGE.0, KELVIN_RANGE.1);
        let edits = parameter_rows::colour_edits(
            &self.target,
            &self.row,
            "blackbody",
            json!({"kind":"floats","value":[kelvin]}),
        );
        self.send_edits(edits, cx);
    }

    /// The light's `scale` beside the temperature: the plain float edit
    /// the field itself would send.
    fn apply_scale(&mut self, value: f64, cx: &mut Context<Self>) {
        let Some(scale) = self.scale.clone() else {
            return;
        };
        let edit = parameter_rows::set_edit(
            &self.target,
            &scale,
            json!({"kind":"floats","value":[value]}),
        );
        self.send_edits(vec![edit], cx);
    }

    /// The slot's own scalar (dielectric's eta): the plain float
    /// set_parameter the resource page's number field would send —
    /// the value stays the parameter's number, never a colour.
    fn apply_number(&mut self, value: f64, cx: &mut Context<Self>) {
        let edits = number_edits(&self.target, &self.row, value);
        self.send_edits(edits, cx);
    }

    /// One tab button: hidden unless the schema's value types allow the
    /// tab, highlighted when the row's value sits in it, and a click on
    /// another replaces the value with the tab type's default (one
    /// Undo step).
    fn tab_button(
        &self,
        tab: SpectrumTab,
        label: &'static str,
        active: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let tabs = parameter_rows::spectrum_tabs(&self.row);
        let allowed = match tab {
            SpectrumTab::Rgb => tabs.rgb,
            SpectrumTab::Blackbody => tabs.blackbody,
            SpectrumTab::Named => tabs.named,
            SpectrumTab::Spectrum => tabs.spectrum,
        };
        if !allowed {
            return None;
        }
        // The current tab keeps its place in the fixed RGB / Blackbody /
        // Spectrum order, highlighted and inert.
        if active {
            return Some(
                Button::new(SharedString::from(format!("{}-tab-{:?}", self.form, tab)))
                    .label(label)
                    .xsmall()
                    .primary()
                    .into_any_element(),
            );
        }
        let button = Button::new(SharedString::from(format!("{}-tab-{:?}", self.form, tab)))
            .label(label)
            .xsmall()
            .on_click(cx.listener(move |this, _, _, cx| {
                let builtins = this.session.read(cx).resource_schema["spectra"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|name| name.as_str().map(str::to_owned))
                    .collect::<Vec<_>>();
                let edits = parameter_rows::tab_default_edits(
                    &this.target,
                    &this.row,
                    tab,
                    &this.items,
                    &builtins,
                );
                if edits.is_empty() {
                    this.message = "No spectrum to give yet — import a .spd file".into();
                    cx.notify();
                } else {
                    this.send_edits(edits, cx);
                }
            }))
            .into_any_element();
        Some(button)
    }
}

impl Render for ColourEdit {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let answer = self.session.update(cx, |session, cx| {
            session.parameter_swatch(
                &self.target,
                &self.row.name,
                parameter_rows::row_role(&self.row).as_deref(),
                cx,
            )
        });
        let current = parameter_rows::current_tab(&self.row);
        // The header: the approximate colour the box shows and the value
        // it names, with the wire's `source` spelling the how. A value
        // with no resolved colour paints the hollow look. The filled
        // muted dark would read as "0".
        let header = {
            let (paint, source) = match &answer {
                Some(answer) if answer.get("error").is_none() => {
                    let channels = |index: usize| {
                        answer["rgb"]
                            .as_array()
                            .and_then(|values| values.get(index))
                            .and_then(Value::as_f64)
                            .unwrap_or_default()
                    };
                    (
                        Some(Rgba {
                            r: channels(0).clamp(0., 1.) as f32,
                            g: channels(1).clamp(0., 1.) as f32,
                            b: channels(2).clamp(0., 1.) as f32,
                            a: 1.,
                        }),
                        answer["source"].as_str().unwrap_or_default().to_owned(),
                    )
                }
                Some(answer) => (
                    None,
                    answer["error"].as_str().unwrap_or_default().to_owned(),
                ),
                None => (
                    None,
                    if self.row.declared {
                        "resolving the colour…".to_owned()
                    } else {
                        "not set".to_owned()
                    },
                ),
            };
            let mut swatch = div().size(px(14.)).flex_shrink_0().rounded_sm();
            swatch = if let Some(paint) = paint {
                swatch.bg(paint)
            } else {
                swatch
                    .overflow_hidden()
                    .child(super::components::unset_swatch())
            };
            div().flex().items_center().gap_2().child(swatch).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(value_text(&self.row, &self.items)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x9098a6))
                            .truncate()
                            .child(source),
                    ),
            )
        };
        // The tabs the schema allows in a fixed RGB / Blackbody / Spectrum
        // order (RGB is the everyday way to set a colour; Spectrum is
        // everything that is neither), the current one highlighted; a click
        // on another replaces the value with that tab's default.
        let tabs = {
            let mut row = div().flex().items_center().gap_1();
            for (tab, label) in [
                (SpectrumTab::Rgb, "RGB"),
                (SpectrumTab::Blackbody, "Blackbody"),
                (SpectrumTab::Named, "Named"),
                (SpectrumTab::Spectrum, "Data"),
            ] {
                if let Some(button) = self.tab_button(tab, label, current == tab, cx) {
                    row = row.child(button);
                }
            }
            row
        };

        // The active tab's body.
        let input = |state: &Entity<InputState>| Input::new(state).xsmall().into_any_element();
        let body = match current {
            SpectrumTab::Rgb => {
                let mut body = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(self.wheel.clone())
                    .child(
                        div().flex().gap_1().children(
                            self.rgb
                                .iter()
                                .map(|state| div().flex_1().min_w_0().child(input(state))),
                        ),
                    );
                // The intensity multiplier — the illuminant rows and
                // any value already wider than 1; a reflectance
                // stays 0..1 without it.
                if intensity_shown(&self.row) {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .w(px(56.))
                                    .text_xs()
                                    .text_color(rgb(0x9098a6))
                                    .child("Intensity"),
                            )
                            .child(div().flex_1().min_w_0().child(input(&self.intensity_input)))
                            .child(div().text_xs().text_color(rgb(0x9098a6)).child("×")),
                    );
                }
                body
            }
            SpectrumTab::Blackbody => {
                let slider = self.kelvin_slider.clone();
                // The temperature as a readout above the slider, following
                // a drag.
                let readout = div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x9098a6))
                            .child("Temperature"),
                    )
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .child(format!("{} K", self.kelvin.read(cx).value())),
                    );
                let mut body = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(readout)
                    .child(Slider::new(&slider));
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(div().flex_1().min_w_0().child(input(&self.kelvin)))
                        .child(div().text_xs().text_color(rgb(0x9098a6)).child("K")),
                );
                // The light's scale beside the temperature.
                if let Some(input_state) = self.scale_input.clone() {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .w(px(24.))
                                    .text_xs()
                                    .text_color(rgb(0x9098a6))
                                    .child("×"),
                            )
                            .child(div().flex_1().min_w_0().child(input(&input_state)))
                            .child(div().text_xs().text_color(rgb(0x9098a6)).child("scale")),
                    );
                }
                body
            }
            SpectrumTab::Named => {
                let named = self.session.read(cx).resource_schema["spectra"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|name| name.as_str().map(str::to_owned))
                    .collect::<Vec<_>>();
                // The row's own family appears first — the
                // conductor's eta/k see the metals, a dielectric's eta
                // the glasses, a light its illuminants — with a toggle
                // to every name the table has; the search
                // filters whichever set is shown.
                let family = parameter_rows::named_family(&self.row);
                let current = match &self.row.field {
                    parameter_rows::Field::Spectrum(name) => Some(name.clone()),
                    _ => None,
                };
                let query = self.named_search.read(cx).value().to_string();
                let shown = named
                    .iter()
                    .filter(|name| {
                        family.is_none_or(|family| self.named_all || family.contains(name))
                    })
                    .filter(|name| matches_search(&query, name))
                    .cloned()
                    .collect::<Vec<_>>();
                let this = cx.entity().downgrade();
                let mut body = div().flex().flex_col().gap_1();
                body = body.child(
                    div()
                        .flex()
                        .gap_1()
                        .items_center()
                        .when_some(family, |row, family| {
                            row.child(
                                Button::new(SharedString::from(format!("{}-named-all", self.form)))
                                    .label(if self.named_all {
                                        format!("Show only {}", family.label())
                                    } else {
                                        format!("Show all ({})", named.len())
                                    })
                                    .xsmall()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.named_all = !this.named_all;
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(div().flex_1().min_w_0().child(input(&self.named_search))),
                );
                if !shown.is_empty() {
                    let form = self.form.clone();
                    let rows = shown
                        .iter()
                        .map(|name| {
                            choice_row(
                                SharedString::from(format!("{form}-named-{name}")),
                                name.clone(),
                                current.as_deref() == Some(name.as_str()),
                                &this,
                                json!({"kind":"builtin_spectra","value":[name]}),
                            )
                        })
                        .collect();
                    body = body.child(choice_list(
                        SharedString::from(format!("{}-named-list", self.form)),
                        rows,
                    ));
                }
                body
            }
            SpectrumTab::Spectrum => {
                let this = cx.entity().downgrade();
                let mut body = div().flex().flex_col().gap_1();
                // The slot's own scalar (dielectric's eta — the
                // declared value or the greyed default it falls back
                // to) keeps its number field: the Data tab is the page
                // such a row opens on, and the switch below still
                // turns it into a spectrum value.
                if number_shown(&self.row) {
                    let number = self.number_input.clone();
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .w(px(40.))
                                    .text_xs()
                                    .text_color(rgb(0x9098a6))
                                    .child("Value"),
                            )
                            .child(div().flex_1().min_w_0().child(input(&number))),
                    );
                }
                // What the row references now, and the way to open it.
                match &self.row.field {
                    parameter_rows::Field::SpectrumData(id) => {
                        let id = id.to_owned();
                        let (name, shown) =
                            (super::components::item_name(&self.items, &id), id.clone());
                        body = body.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(div().flex_1().min_w_0().truncate().child(name))
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "{}-open-spectrum",
                                        self.form
                                    )))
                                    .icon(assets::IconName::SquarePen)
                                    .xsmall()
                                    .tooltip("Open in Editor")
                                    .on_click(
                                        move |_, window, cx| {
                                            window.dispatch_action(
                                                Box::new(super::menu::OpenInEditor {
                                                    id: shown.clone(),
                                                }),
                                                cx,
                                            )
                                        },
                                    ),
                                ),
                        );
                    }
                    // Inline wavelength/value pairs
                    // keep the array editor they had before the dialog.
                    parameter_rows::Field::Json => {
                        if let Some(table) = super::parameter_values::table(&self.row) {
                            let (target, name) = (self.target.clone(), self.row.name.clone());
                            let read_only = self.session.read(cx).document().is_read_only();
                            body =
                                body.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(rgb(0xc4c8d0))
                                                .child(table.summary()),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "{}-open-array-editor",
                                                self.form
                                            )))
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
                                        ),
                                );
                        }
                    }
                    _ => {}
                }
                // The project's SpectrumData resources, listed and
                // searched inside the dialog because an outside menu dies
                // with the popover before its click lands.
                let choices = spectrum_data_choices(&self.items);
                if !choices.is_empty() {
                    let query = self.data_search.read(cx).value().to_string();
                    let shown = choices
                        .iter()
                        .filter(|(name, _)| matches_search(&query, name))
                        .cloned()
                        .collect::<Vec<_>>();
                    let mut picker = div().flex().flex_col().gap_1();
                    let search = self.data_search.clone();
                    picker = picker.child(div().flex_1().min_w_0().child(input(&search)));
                    let current = match &self.row.field {
                        parameter_rows::Field::SpectrumData(id) => Some(id.clone()),
                        _ => None,
                    };
                    if !shown.is_empty() {
                        let form = self.form.clone();
                        let rows = shown
                            .iter()
                            .map(|(name, id)| {
                                choice_row(
                                    SharedString::from(format!("{form}-data-{id}")),
                                    name.clone(),
                                    current.as_deref() == Some(id.as_str()),
                                    &this,
                                    json!({"kind":"resources","value":{"kind":"spectrum_data","ids":[id]}}),
                                )
                            })
                            .collect();
                        picker = picker.child(choice_list(
                            SharedString::from(format!("{}-data-list", self.form)),
                            rows,
                        ));
                    }
                    body = body.child(picker);
                }
                body.child(
                    Button::new(SharedString::from(format!("{}-import-spd", self.form)))
                        .label("Import .spd…")
                        .xsmall()
                        .when(
                            self.session.read(cx).project.context.project.is_none(),
                            |button| button.disabled(true),
                        )
                        .on_click(|_, window, cx| {
                            window
                                .dispatch_action(Box::new(super::menu::ContentsImportResource), cx)
                        }),
                )
            }
        };

        let feedback = self.message.clone();
        let message = self
            .session
            .read(cx)
            .edit_feedback
            .message(&self.form)
            .map(str::to_owned);
        div()
            .w(px(280.))
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .child(tabs)
            .child(body)
            .when_some(message, |this, message| {
                this.child(div().text_color(rgb(0xd99a74)).child(message))
            })
            .when(!feedback.is_empty(), |this| {
                this.child(div().text_color(rgb(0xd99a74)).child(feedback))
            })
    }
}

#[cfg(test)]
mod tests {
    // Specific imports, not `use super::*`: the glob would chain through
    // gpui-kit's exports and shadow `#[test]` itself.
    use super::{
        KELVIN_RANGE, OpenDialogs, field_number, fraction_kelvin, illuminant_row, intensity_shown,
        kelvin_fraction, matches_search, merge_intensity, number_edits, number_shown, rgb_edits,
        row_number, scale_number, spectrum_data_choices, split_intensity, value_text,
    };
    use serde_json::{Value, json};

    /// The blackbody slider is logarithmic in kelvin,
    /// both directions over its own 1000–40000 K span — a fixed track
    /// fraction moves the temperature by a fixed ratio, so a 30 px drag
    /// no longer takes 5500 K down to 1400 K.
    #[test]
    fn the_kelvin_slider_maps_logarithmically_both_ways() {
        // The span ends: 0 and 1, named exactly from the positions; the
        // kelvins come back to the whole number within the exp/ln
        // roundoff.
        assert_eq!(kelvin_fraction(KELVIN_RANGE.0), 0.);
        assert_eq!(kelvin_fraction(KELVIN_RANGE.1), 1.);
        assert!((fraction_kelvin(1.) - KELVIN_RANGE.1).abs() < 1e-6);
        assert!((fraction_kelvin(0.) - KELVIN_RANGE.0).abs() < 1e-6);
        // The midpoint is the geometric middle, not the arithmetic one.
        let middle = (KELVIN_RANGE.0 * KELVIN_RANGE.1).sqrt();
        assert!(
            (fraction_kelvin(0.5) - middle).abs() < 1e-6,
            "{}",
            fraction_kelvin(0.5)
        );
        assert!((kelvin_fraction(middle) as f64 - 0.5).abs() < 1e-6);
        // A temperature beyond the span clamps into it, both directions.
        assert_eq!(kelvin_fraction(500.), 0.);
        assert_eq!(kelvin_fraction(50000.), 1.);
        assert!((fraction_kelvin(1.5) - KELVIN_RANGE.1).abs() < 1e-6);
        assert!((fraction_kelvin(-0.5) - KELVIN_RANGE.0).abs() < 1e-6);
        // Round trips agree both ways, closer than a f32 track position.
        for kelvin in [1000., 1500., 2700., 5500., 6500., 12000., 30000., 39500.] {
            let back = fraction_kelvin(kelvin_fraction(kelvin) as f64);
            assert!(
                ((back - kelvin) / kelvin).abs() < 1e-4,
                "{kelvin} comes back {back}"
            );
            let position = kelvin_fraction(kelvin);
            let forth = kelvin_fraction(fraction_kelvin(position as f64));
            assert!(
                (forth - position).abs() < 1e-5,
                "{position} comes back {forth}"
            );
        }
        // The logarithmic property itself: the same track step scales
        // the temperature by the same factor wherever it is taken.
        let ratio = |kelvin: f64, step: f64| {
            fraction_kelvin(kelvin_fraction(kelvin) as f64 + step) / kelvin
        };
        assert!((ratio(2000., 0.1) - 40_f64.powf(0.1)).abs() < 1e-5);
        assert!((ratio(20000., 0.1) - 40_f64.powf(0.1)).abs() < 1e-5);
        assert!((ratio(5500., -0.1) * 40_f64.powf(0.1) - 1.).abs() < 1e-5);
        // The linear drag: 30 px of the dialog's ~270 px track, an 11%
        // step, moved 5500 K to 1400 K; logarithmic it reaches ~3670 K
        // — coarse, not a plunge off the scale.
        let dragged = fraction_kelvin(kelvin_fraction(5500.) as f64 - 0.11).round();
        assert!((dragged - 3670.).abs() < 30., "{dragged}");
    }

    /// (item 3): the search of an in-dialog list filters by a
    /// case-insensitive substring — built-in spectra and Spectrum Data
    /// names alike; an empty (or blank) query matches everything.
    #[test]
    fn search_matches_labels_by_case_insensitive_substring() {
        assert!(matches_search("", "stdillum-D65"));
        assert!(matches_search("   ", "stdillum-D65"));
        assert!(matches_search("d65", "stdillum-D65"));
        assert!(matches_search("ILLUM", "stdillum-D65"));
        assert!(matches_search(" Metal-Cu-ETA ", "metal-Cu-eta"));
        assert!(matches_search("spd", "Au.k.spd"));
        assert!(!matches_search("ag", "metal-Cu-eta"));
        assert!(!matches_search("cu-eta", "stdillum-D65"));
    }

    /// (item 6): the open-dialog bookkeeping by form id — the
    /// popover's own open state dies with a rebuilt field, so the
    /// session remembers the dialogs their users had open. One form's
    /// flag never leaks to another, and only a dismissal clears one.
    #[test]
    fn open_dialog_bookkeeping_keeps_each_form_until_closed() {
        let mut dialogs = OpenDialogs::default();
        assert!(!dialogs.is_open("parameter:m:eta"));
        dialogs.set_open("parameter:m:eta", true);
        assert!(dialogs.is_open("parameter:m:eta"));
        assert!(
            !dialogs.is_open("parameter:m:k"),
            "one field's dialog is its own"
        );
        dialogs.set_open("parameter:m:k", true);
        dialogs.set_open("parameter:m:eta", false);
        assert!(
            !dialogs.is_open("parameter:m:eta"),
            "the dismissal clears it"
        );
        assert!(
            dialogs.is_open("parameter:m:k"),
            "another form's dialog stays open"
        );
        dialogs.set_open("parameter:m:eta", true);
        assert!(
            dialogs.is_open("parameter:m:eta"),
            "a reopened dialog reports itself again"
        );
    }

    /// (item 5): the header names the row's value the way a
    /// person reads it — the resource's name and its kind, not the
    /// wire's spelling; an unset row keeps its hint.
    #[test]
    fn headers_name_each_value_kind() {
        let items = json!({"sd-1":{"resource_kind":"spectrum_data","name":"Au.eta.spd"}});
        let spectrum_slot = spec("k", json!(["spectrum", "rgb"]), Some("spectrum_unbounded"));
        // Spectrum Data: the resource's name, and the kind.
        let declared_reference = json!({
            "name":"k","value_type":"spectrum","color_space":{"kind":"inherit"},
            "values":{"kind":"resources","resource_kind":"spectrum_data","ids":["sd-1"]},
            "preview_truncated":false});
        let row = &rows_of(&spectrum_slot, &[declared_reference])[0];
        assert_eq!(value_text(row, &items), "Au.eta.spd · Spectrum Data");
        // Named: the spectrum's name without the wire's quotes.
        let named = json!({
            "name":"k","value_type":"spectrum","color_space":{"kind":"inherit"},
            "values":{"kind":"builtin_spectra","value":["stdillum-D65"]},
            "preview_truncated":false});
        let row = &rows_of(&spectrum_slot, &[named])[0];
        assert_eq!(value_text(row, &items), "stdillum-D65 · Named");
        // Blackbody: the temperature a person would say.
        let kelvin = &rows_of(
            &light_spec(),
            &[declared(
                "I",
                "blackbody",
                json!({"kind":"inherit"}),
                json!([5500.]),
            )],
        )[0];
        assert_eq!(value_text(kelvin, &items), "Blackbody 5500 K");
        // RGB: the three channels, two decimals.
        let colour = &rows_of(
            &light_spec(),
            &[declared(
                "I",
                "rgb",
                json!({"kind":"inherit"}),
                json!([0.5, 0.5, 0.5]),
            )],
        )[0];
        assert_eq!(value_text(colour, &items), "RGB 0.50, 0.50, 0.50");
        // A constant float spectrum keeps its number.
        let eta = json!({"name":"eta","aliases":[],"value_types":["float","spectrum"],
                         "default":{"value_type":"float","values":[1.5]},
                         "default_kind":"none","default_depends":null,
                         "texture":null,"role":"texture_input","required":false});
        let declared_eta = json!({"name":"eta","value_type":"float",
            "color_space":{"kind":"inherit"},
            "values":{"kind":"floats","value":[1.5]},"preview_truncated":false});
        let scalar = &rows_of(&eta, &[declared_eta])[0];
        assert_eq!(value_text(scalar, &items), "1.5");
        // An unset row keeps the hint it shows today.
        let unset = &rows_of(&light_spec(), &[])[0];
        assert_eq!(value_text(unset, &items), "(not set)");
    }

    #[test]
    fn rgb_values_split_into_colour_and_intensity_and_carry_back() {
        // In range: the colour carries itself at intensity 1.
        assert_eq!(split_intensity([0.5, 0.25, 0.75]), ([0.5, 0.25, 0.75], 1.));
        assert_eq!(split_intensity([1., 1., 1.]), ([1., 1., 1.], 1.));
        assert_eq!(split_intensity([0., 0., 0.]), ([0., 0., 0.], 1.));
        // The biggest channel over 1 becomes the intensity; the colour
        // is the value scaled down to it.
        assert_eq!(split_intensity([2., 1., 0.]), ([1., 0.5, 0.], 2.));
        assert_eq!(split_intensity([3., 1.5, 0.75]), ([1., 0.5, 0.25], 3.));
        // A mix keeps its sign through the split.
        assert_eq!(split_intensity([5., -2., 0.]), ([1., -0.4, 0.], 5.));
        // The merge is the exact inverse, over every split.
        for rgb in [
            [0.5, 0.25, 0.75],
            [1., 1., 1.],
            [0., 0., 0.],
            [2., 1., 0.],
            [3., 1.5, 0.75],
            [5., -2., 0.],
        ] {
            let (colour, intensity) = split_intensity(rgb);
            assert_eq!(merge_intensity(colour, intensity), rgb, "{rgb:?}");
        }
        // And the plain merge scales whatever colour it is given.
        assert_eq!(
            merge_intensity([1., 0.5, 0.25], 2.25),
            [2.25, 1.125, 0.5625]
        );
        assert_eq!(merge_intensity([1., 1., 1.], 1.), [1., 1., 1.]);
    }

    /// The rows of a schema page, from the same wire shapes the
    /// Inspector builds them.
    fn rows_of(spec: &Value, parameters: &[Value]) -> Vec<super::Row> {
        super::super::parameter_rows::rows(&json!({
            "parameters": parameters,
            "schema": [spec],
        }))
    }

    fn spec(name: &str, value_types: Value, texture: Option<&str>) -> Value {
        json!({"name":name,"aliases":[],"value_types":value_types,"default":null,
               "default_kind":"none","default_depends":null,
               "texture":texture.map(|kind| json!({"kind":kind})),
               "role":"texture_input","required":false})
    }

    fn declared(name: &str, value_type: &str, color_space: Value, value: Value) -> Value {
        json!({"name":name,"value_type":value_type,"color_space":color_space,
               "values":{"kind":"floats","value":value},"preview_truncated":false})
    }

    fn light_spec() -> Value {
        spec("I", json!(["spectrum", "rgb", "blackbody"]), None)
    }

    #[test]
    fn illuminant_rows_and_wide_values_show_the_intensity_field() {
        // A light's I: the table allows blackbody, the intensity's role.
        let light = &rows_of(&light_spec(), &[])[0];
        assert!(illuminant_row(light));
        assert!(intensity_shown(light));
        // A texture input of the schema's spectrum_illuminant kind.
        let glow = &rows_of(
            &spec(
                "glow",
                json!(["spectrum", "rgb"]),
                Some("spectrum_illuminant"),
            ),
            &[],
        )[0];
        assert!(illuminant_row(glow));
        assert!(intensity_shown(glow));
        // A reflectance stays 0..1: no multiplier for it — until a
        // value wider than 1 sits in the row.
        let albedo = &spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Some("spectrum_albedo"),
        );
        let flat = &rows_of(
            albedo,
            &[declared(
                "reflectance",
                "rgb",
                json!({"kind":"inherit"}),
                json!([0.5, 0.25, 0.5]),
            )],
        )[0];
        assert!(!illuminant_row(flat));
        assert!(!intensity_shown(flat));
        let wide = &rows_of(
            albedo,
            &[declared(
                "reflectance",
                "rgb",
                json!({"kind":"inherit"}),
                json!([2., 1., 0.5]),
            )],
        )[0];
        assert!(!illuminant_row(wide));
        assert!(intensity_shown(wide), "the value over 1 needs the carry");
    }

    #[test]
    fn intensity_edits_leave_as_one_wire_rgb_carry() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-00000000aa01"});
        // The dialog's one apply: the merged colour rides the rgb edit,
        // the explicit colour space kept on the rgb-to-rgb carry.
        let row = &rows_of(
            &light_spec(),
            &[declared(
                "I",
                "rgb",
                json!({"kind":"explicit","value":"aces2065-1"}),
                json!([1.5, 1.5, 1.5]),
            )],
        )[0];
        let edit = rgb_edits(&target, row, merge_intensity([0.75, 0.5, 0.25], 2.));
        assert_eq!(
            edit,
            vec![
                json!({"kind":"set_parameter","target":target,"name":"I","value_type":"rgb",
                     "value":{"name":"I","value_type":"rgb","color_space":"aces2065-1",
                              "values":{"kind":"floats","value":[1.5, 1., 0.5]}}})
            ]
        );
        // Every edit the RGB tab builds deserialises as the real wire
        // edit type (the earlier stages' lesson: JSON shape alone
        // proved nothing).
        for edit in &edit {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
        // A cross-type carry — a blackbody-declared light whose value
        // the RGB tab gives a colour — replaces the declaration first
        // and both edits ride the one transaction.
        let kelvin = &rows_of(
            &light_spec(),
            &[declared(
                "I",
                "blackbody",
                json!({"kind":"inherit"}),
                json!([6500.]),
            )],
        )[0];
        let edits = rgb_edits(&target, kelvin, [2.4, 1.2, 0.6]);
        assert_eq!(
            edits,
            vec![
                json!({"kind":"set_parameter","target":target,"name":"I","value_type":"blackbody",
                        "value":null}),
                json!({"kind":"set_parameter","target":target,"name":"I","value_type":"rgb",
                        "value":{"name":"I","value_type":"rgb","color_space":null,
                                 "values":{"kind":"floats","value":[2.4, 1.2, 0.6]}}}),
            ]
        );
        for edit in edits {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
    }

    #[test]
    fn field_numbers_trim_trailing_zeros() {
        assert_eq!(field_number(6500.), "6500");
        assert_eq!(field_number(0.25), "0.25");
        assert_eq!(field_number(-0.), "0");
    }

    /// A float-declared spectrum slot (dielectric's eta, the finding's
    /// instance) keeps a scalar editor on its Data tab: the field shows
    /// the declared number or the greyed default, and its edit is the
    /// owner's own float set_parameter — never a colour.
    #[test]
    fn float_rows_edit_their_scalar_into_the_owners_declaration() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        // The dielectric's eta: float and spectrum, no rgb — the Data
        // tab is the page it opens on, and the scalar is what it edits.
        let eta = json!({"name":"eta","aliases":[],"value_types":["float","spectrum"],
                         "default":{"value_type":"float","values":[1.5]},
                         "default_kind":"none","default_depends":null,
                         "texture":null,"role":"texture_input","required":false});
        let eta_row = &rows_of(
            &eta,
            &[declared(
                "eta",
                "float",
                json!({"kind":"inherit"}),
                json!([1.5]),
            )],
        )[0];
        assert!(number_shown(eta_row), "a declared float shows its scalar");
        assert_eq!(row_number(eta_row), "1.5");
        let edits = number_edits(&target, eta_row, 1.7);
        assert_eq!(
            edits,
            vec![json!({"kind":"set_parameter","target":target,"name":"eta",
                   "value_type":"float",
                   "value":{"name":"eta","value_type":"float","color_space":null,
                            "values":{"kind":"floats","value":[1.7]}}})]
        );
        serde_json::from_value::<protocol::project::ProjectEdit>(edits[0].clone())
            .unwrap_or_else(|error| panic!("{error}: {}", edits[0]));
        // The greyed default shows and edits the same way — the page is
        // the slot's editor even after an undo removed the declaration.
        let greyed = &rows_of(&eta, &[])[0];
        assert!(number_shown(greyed));
        assert_eq!(row_number(greyed), "1.5");
        let edits = number_edits(&target, greyed, 1.2);
        assert_eq!(
            edits,
            vec![json!({"kind":"set_parameter","target":target,"name":"eta",
                   "value_type":"float",
                   "value":{"name":"eta","value_type":"float","color_space":null,
                            "values":{"kind":"floats","value":[1.2]}}})],
            "an undeclared slot declares under the table's own spelling"
        );
        serde_json::from_value::<protocol::project::ProjectEdit>(edits[0].clone())
            .unwrap_or_else(|error| panic!("{error}: {}", edits[0]));
        // The shapes with editors of their own do not take the scalar
        // field: an rgb triple, a blackbody temperature, and a plain
        // float slot without a spectrum value.
        let rgb = &rows_of(
            &spec("k", json!(["spectrum", "rgb"]), Some("spectrum_albedo")),
            &[declared(
                "k",
                "rgb",
                json!({"kind":"inherit"}),
                json!([1., 0.5, 0.25]),
            )],
        )[0];
        assert!(!number_shown(rgb));
        let kelvin = &rows_of(
            &spec("I", json!(["spectrum", "rgb", "blackbody"]), None),
            &[declared(
                "I",
                "blackbody",
                json!({"kind":"inherit"}),
                json!([6500.]),
            )],
        )[0];
        assert!(
            !number_shown(kelvin),
            "the temperature keeps its own editor"
        );
        // A float slot without a spectrum value never opens this page:
        // it grows no value node and no colour dialog of its own.
        let plain = &rows_of(&spec("vroughness", json!(["float"]), None), &[])[0];
        assert!(
            !super::parameter_rows::spectrum_tabs(plain).spectrum,
            "no Data tab without a spectrum value"
        );
    }

    #[test]
    fn spectrum_data_choices_list_sorted_by_name() {
        let items = json!({
            "00000000-0000-0000-0000-00000000ab01":{"resource_kind":"spectrum_data","name":"b-side.spd"},
            "00000000-0000-0000-0000-00000000ab00":{"resource_kind":"spectrum_data","name":"a-side.spd"},
            "m":{"resource_kind":"material","name":"zebra"},
        });
        assert_eq!(
            spectrum_data_choices(&items),
            vec![
                (
                    "a-side.spd".to_owned(),
                    "00000000-0000-0000-0000-00000000ab00".to_owned()
                ),
                (
                    "b-side.spd".to_owned(),
                    "00000000-0000-0000-0000-00000000ab01".to_owned()
                ),
            ]
        );
        assert!(spectrum_data_choices(&json!({})).is_empty());
    }

    #[test]
    fn scale_numbers_come_from_the_rows_field() {
        use super::super::parameter_rows::{Field, Row};
        let mut row = Row {
            name: "scale".into(),
            value_type: "float".into(),
            implementation: String::new(),
            declared: false,
            known: true,
            required: false,
            texture: None,
            texture_kind: None,
            value_types: vec!["float".into()],
            file: false,
            group: None,
            when: Vec::new(),
            choices: Vec::new(),
            suggestions: Vec::new(),
            field: Field::Number {
                integer: false,
                value: "1".into(),
            },
            ignored: None,
            hint: String::new(),
            parameter: serde_json::Value::Null,
        };
        assert_eq!(scale_number(&row), "1");
        row.field = Field::Number {
            integer: false,
            value: "2.5".into(),
        };
        assert_eq!(scale_number(&row), "2.5");
        row.field = Field::Unset;
        assert_eq!(
            scale_number(&row),
            "1",
            "a scale row without a number still offers 1"
        );
    }
}

#[cfg(test)]
mod slider_tests {
    use super::kelvin_slider_state;

    /// Building the slider must not panic (it did, on `min(1000)`
    /// once), and its 0..1 track is the logarithmic kelvin mapping's
    /// position space; values past the span clamp into
    /// it inside `kelvin_fraction`.
    #[test]
    fn the_kelvin_slider_builds_for_any_start_value() {
        for kelvin in [0., 500., 6500., 40000., 100000.] {
            let slider = kelvin_slider_state(kelvin);
            assert_eq!(slider.min_value(), 0., "{kelvin}");
            assert_eq!(slider.max_value(), 1., "{kelvin}");
        }
    }
}
