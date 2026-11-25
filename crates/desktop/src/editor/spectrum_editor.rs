//! The Spectrum Data Editor: the
//! resource's curve with the visible band behind it, the approximate
//! colours `project.spectrum_color` resolves, and the two-column sample
//! table. Table edits go out as one `project.apply` transaction when the
//! tables are inline floats — one Undo step; `.array`-backed tables keep
//! their length and edit through `project.set_parameter_values`.
//! The curve is a preview: no drag editing.
use super::Session;
use crate::backend::{Command, Follow};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::{
    button::Button,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

/// The rows one table page shows.
pub(super) const PAGE_ROWS: usize = 100;

/// item 7: one sample row's height — the 23 px inputs plus the
/// half-step gap between them.
const ROW_HEIGHT_PX: f32 = 25.;
/// The minimum rows a short dock leaves visible —
/// "page 1 of 1" above a zero-row table hid the samples entirely.
const MIN_TABLE_ROWS: usize = 6;
/// The sample table's minimum height: [`MIN_TABLE_ROWS`] rows.
const MIN_TABLE_PX: f32 = MIN_TABLE_ROWS as f32 * ROW_HEIGHT_PX;
/// The curve plot's preferred height.
const PLOT_PREFERRED: f32 = 150.;
/// The curve plot's minimum height — the plot gives ground before the
/// table's rows do.
const PLOT_MIN: f32 = 60.;

/// The maximum sample-pair count is `project::MAX_SPECTRUM_SAMPLES`.
const MAX_SAMPLES: usize = project::MAX_SPECTRUM_SAMPLES;

/// The most values one wire edit may carry: core's `project.apply`
/// refuses a set_parameter with more values (edits.rs), and one
/// `project.set_parameter_values` window holds 1 to this many.
pub(super) const WIRE_WINDOW_VALUES: usize = 4096;

/// One draft row: the editable texts of a wavelength and its value.
type Row = (String, String);

/// Parse the draft rows into the two float tables; the answer names the
/// rows that do not parse (so the UI can paint them red): `Ok` requires
/// every row to hold two finite floats.
pub(super) fn parse_rows(draft: &[Row]) -> Result<(Vec<f64>, Vec<f64>), Vec<usize>> {
    let mut wavelengths = Vec::with_capacity(draft.len());
    let mut values = Vec::with_capacity(draft.len());
    let mut bad = Vec::new();
    for (index, (wavelength, value)) in draft.iter().enumerate() {
        let pair = (
            wavelength.trim().parse::<f64>().ok(),
            value.trim().parse::<f64>().ok(),
        );
        match pair {
            (Some(wavelength), Some(value)) if wavelength.is_finite() && value.is_finite() => {
                wavelengths.push(wavelength);
                values.push(value);
            }
            _ => bad.push(index),
        }
    }
    if bad.is_empty() {
        Ok((wavelengths, values))
    } else {
        Err(bad)
    }
}

/// Whether the parsed tables are a spectrum validation accepts: at least
/// one pair, at most `MAX_SPECTRUM_SAMPLES`, equal counts, and strictly
/// ascending positive wavelengths. The message is the reviewer's hint,
/// naming the first bad row.
pub(super) fn validate_tables(wavelengths: &[f64], values: &[f64]) -> Result<(), String> {
    if wavelengths.len() != values.len() {
        return Err("wavelengths and values must hold the same number of samples".into());
    }
    if wavelengths.is_empty() {
        return Err("the spectrum needs at least one sample".into());
    }
    if wavelengths.len() > MAX_SAMPLES {
        return Err(format!(
            "too many samples: {MAX_SAMPLES} is the upper bound"
        ));
    }
    for (index, wavelength) in wavelengths.iter().enumerate() {
        if *wavelength <= 0. {
            return Err(format!("row {index}: wavelengths must be positive"));
        }
        // Equal neighbours are valid (the project model and r4 only
        // reject descending wavelengths).
        if index > 0 && *wavelength < wavelengths[index - 1] {
            return Err(format!("row {index}: wavelengths must not descend"));
        }
    }
    Ok(())
}

/// Clipboard text to rows: lines of `λ v` pairs, tab or space separated,
/// empty lines skipped. The texts are kept verbatim for the cells.
pub(super) fn paste_rows(text: &str) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(wavelength), Some(value), None) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(format!("line {}: expected \"wavelength value\"", index + 1));
        };
        // Numbers only: pasted rows land ready to edit, not as red cells.
        for text in [wavelength, value] {
            if !text.parse::<f64>().is_ok_and(f64::is_finite) {
                return Err(format!("line {}: expected \"wavelength value\"", index + 1));
            }
        }
        rows.push((wavelength.to_owned(), value.to_owned()));
    }
    Ok(rows)
}

/// The one-transaction apply edits for inline tables: `set_parameter`
/// for `wavelengths` and `values`, plain wire floats.
pub(super) fn set_parameter_edits(
    target: &Value,
    wavelengths: &[f64],
    values: &[f64],
) -> Vec<Value> {
    [("wavelengths", wavelengths), ("values", values)]
        .into_iter()
        .map(|(name, floats)| {
            json!({"kind":"set_parameter","target":target,"name":name,"value_type":"float",
                   "value":{"name":name,"value_type":"float","color_space":null,
                            "values":{"kind":"floats","value":floats}}})
        })
        .collect()
}

/// The `project.set_parameter_values` params that rewrite one table's
/// whole (unchanged-length) content, in contiguous windows of at most
/// `WIRE_WINDOW_VALUES` values each: the `.array` tables the editor
/// opens exceed that limit by construction (the bulk threshold is what
/// created them), so one window could never carry them.
pub(super) fn value_window_params(
    project: &Value,
    target: &Value,
    name: &str,
    values: &[f64],
) -> Value {
    let windows = values
        .chunks(WIRE_WINDOW_VALUES)
        .enumerate()
        .map(|(index, chunk)| {
            json!({"offset": index * WIRE_WINDOW_VALUES,
                   "values": {"kind":"floats","value": chunk}})
        })
        .collect::<Vec<_>>();
    json!({"project":project,"target":target,"name":name,"windows":windows})
}

/// How one apply reaches the wire (review issues 1+2): the
/// one-undo-step transaction while both tables fit the per-edit limit,
/// or fixed-length chunked value windows otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WriteRoute {
    /// set_parameter for both tables in one project.apply — one undo step.
    ApplyBothTables,
    /// project.set_parameter_values per table, windows of at most
    /// `WIRE_WINDOW_VALUES` values rewriting the same length.
    ValueWindows,
}

/// The route this draft's apply must take. `.array` storage rewrites
/// only through value windows (apply cannot patch data assets), and an
/// inline table with more than `WIRE_WINDOW_VALUES` rows cannot ride a
/// set_parameter edit (both tables would exceed the per-edit value
/// limit), so it keeps its fixed length and takes the windows too.
pub(super) fn write_route(storage: &str, draft: &[Row]) -> WriteRoute {
    if storage == "array" || draft.len() > WIRE_WINDOW_VALUES {
        WriteRoute::ValueWindows
    } else {
        WriteRoute::ApplyBothTables
    }
}

/// Whether add/delete/paste may change the row count here: only inline
/// tables under the per-edit limit can — window rewrites keep their
/// length, and one row more would overflow the transaction that
/// length-changing edits depend on.
pub(super) fn row_edits_allowed(storage: &str, draft: &[Row]) -> bool {
    storage != "array" && draft.len() < WIRE_WINDOW_VALUES
}

/// What a fresh table payload means for the local draft (review issue 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FreshPayload {
    /// Replace the draft with the payload's rows.
    Adopt,
    /// Keep the unsaved typing; a message says the change landed.
    KeepDraft,
}

/// A clean draft (nothing typed since the last payload) adopts whatever
/// the project answers, and so does the payload of our own apply — the
/// cells may spell the values less canonically than the payload renders
/// them, so the comparison goes through the parsed samples. A foreign
/// change under local typing keeps every unsaved row.
pub(super) fn fresh_payload_action(draft: &[Row], saved: &[Row], payload: &[Row]) -> FreshPayload {
    if draft == saved || rows_same_samples(draft, payload) {
        FreshPayload::Adopt
    } else {
        FreshPayload::KeepDraft
    }
}

/// Whether two row lists hold the same λ/value samples: parsed as
/// floats when both parse, else only when the texts match verbatim.
fn rows_same_samples(left: &[Row], right: &[Row]) -> bool {
    match (parse_rows(left), parse_rows(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// A fetched table's editable contents.
#[derive(Debug)]
pub(super) struct LoadedTable {
    pub(super) rows: Vec<Row>,
    pub(super) curve: Vec<(f64, f64)>,
    /// "floats" or "array": which write path the tables live in.
    pub(super) storage: String,
}

/// The draft rows' validation outcome for one render.
pub(super) struct DraftState {
    /// The parsed tables when every row parses and validation accepts.
    pub(super) tables: Option<(Vec<f64>, Vec<f64>)>,
    /// The rows whose text does not parse.
    pub(super) red_rows: Vec<usize>,
    /// The first problem to tell the editor, empty when healthy.
    pub(super) message: String,
}

impl DraftState {
    fn rejected(red_rows: Vec<usize>, message: String) -> Self {
        Self {
            tables: None,
            red_rows,
            message,
        }
    }
}

/// A fetched table (`Update::SpectrumTable`'s payload) into draft rows
/// and the saved curve, or the fetch error's message.
pub(super) fn rows_from_table(table: &Value) -> Result<LoadedTable, String> {
    if let Some(error) = table.get("error").and_then(Value::as_str) {
        return Err(error.to_owned());
    }
    let floats = |name: &str| -> Result<Vec<f64>, String> {
        table[name]
            .as_array()
            .ok_or_else(|| format!("the table has no {name} samples"))?
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .ok_or_else(|| format!("a {name} sample is not a finite number"))
            })
            .collect()
    };
    let wavelengths = floats("wavelengths")?;
    let values = floats("values")?;
    let rows = wavelengths
        .iter()
        .zip(&values)
        .map(|(wavelength, value)| (cell_number(*wavelength), cell_number(*value)))
        .collect();
    let curve = wavelengths.into_iter().zip(values).collect();
    Ok(LoadedTable {
        rows,
        curve,
        storage: table["storage"].as_str().unwrap_or("floats").to_owned(),
    })
}

/// The plot's axis ranges from the loaded curve: `(λ min, λ max, y
/// max)`, padded and with fallbacks for degenerate curves.
pub(super) fn curve_axes(samples: &[(f64, f64)]) -> (f64, f64, f64) {
    let first = samples.first().map(|pair| pair.0).unwrap_or_default();
    let (mut min, mut max) = (first, first);
    let mut y_max: f64 = 0.;
    for (wavelength, value) in samples {
        min = min.min(*wavelength);
        max = max.max(*wavelength);
        y_max = y_max.max(*value);
    }
    // One sample, or a flat zero curve, still spans a visible plot.
    if (max - min) < 1e-9 {
        min -= 50.;
        max += 50.;
    }
    if y_max < 1e-9 {
        y_max = 1.;
    }
    (min, max, y_max)
}

/// The visible band's faint background colours: one `[λ, rgb]` per band,
/// the CIE chromaticity of a narrow spike at `λ`.
pub(super) fn visible_bands() -> &'static Vec<(f64, [f64; 3])> {
    use std::sync::OnceLock;
    static BANDS: OnceLock<Vec<(f64, [f64; 3])>> = OnceLock::new();
    BANDS.get_or_init(|| {
        (38..=78)
            .map(|step| {
                let wavelength = step as f64 * 10.;
                // A narrow spike: zero, one, zero around the band centre,
                // resolved through the crate the swatches share.
                let rgb = spectrum::pairs_rgb(
                    &[wavelength - 1., 0., wavelength, 1., wavelength + 1., 0.],
                    spectrum::Role::Unbounded,
                )
                .unwrap_or([1., 1., 1.]);
                (wavelength, rgb)
            })
            .collect()
    })
}

/// A number the cell texts keep: whole values without a trailing ".0".
pub(super) fn cell_number(value: f64) -> String {
    if value == value.trunc() && value.is_finite() && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

pub struct SpectrumEditor {
    session: Entity<Session>,
    /// The Contents entry being edited; `entry["id"]` is the resource.
    entry: Value,
    target: Value,
    /// The apply feedback form id.
    form: String,
    focus: FocusHandle,
    /// The current table page (`PAGE_ROWS` rows).
    page: usize,
    /// The editable texts, the whole table in memory.
    draft: Vec<Row>,
    /// The rows as loaded, to tell an edit from the saved state.
    saved: Vec<Row>,
    /// The saved curve the plot falls back to while a draft is invalid.
    saved_curve: Vec<(f64, f64)>,
    /// "floats" or "array": which write path the tables live in.
    storage: String,
    /// The last table payload applied to the draft, and a generation
    /// counter the input rebuild keys on.
    loaded: Option<Value>,
    load_generation: usize,
    inputs: Vec<Entity<InputState>>,
    input_key: String,
    /// Local validation paste and fetch errors; shown in red.
    message: String,
}

/// The curve over the faint visible band, drawn in whatever bounds it
/// gets: the editor's plot and the Contents row thumbnail share this.
pub(super) fn paint_curve(bounds: Bounds<Pixels>, curve: &[(f64, f64)], window: &mut Window) {
    let axes = curve_axes(curve);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let x_of = |wavelength: f64| -> f32 {
        ((wavelength - axes.0) as f32 / (axes.1 - axes.0) as f32).clamp(0., 1.) * width
    };
    let y_of = |value: f64| -> f32 {
        height - 7. - ((value / axes.2) as f32).clamp(0., 1.) * (height - 14.)
    };
    let fill_rect = |window: &mut Window, x0: f32, x1: f32, y0: f32, y1: f32, color: Rgba| {
        let mut path = PathBuilder::fill();
        path.add_polygon(
            &[
                point(bounds.origin.x + px(x0), bounds.origin.y + px(y0)),
                point(bounds.origin.x + px(x1), bounds.origin.y + px(y0)),
                point(bounds.origin.x + px(x1), bounds.origin.y + px(y1)),
                point(bounds.origin.x + px(x0), bounds.origin.y + px(y1)),
            ],
            true,
        );
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    };
    // The faint visible band behind the curve.
    for (wavelength, rgb) in visible_bands() {
        let x0 = x_of(*wavelength);
        let x1 = x_of(wavelength + 10.).max(x0 + 0.5);
        let peak = rgb.iter().copied().fold(0f64, f64::max);
        let scale = if peak > 1e-9 { 1. / peak } else { 0. };
        let tint = |channel: f64| (channel * scale) as f32;
        fill_rect(
            window,
            x0,
            x1,
            6.,
            height - 6.,
            Rgba {
                r: tint(rgb[0]),
                g: tint(rgb[1]),
                b: tint(rgb[2]),
                a: 0.22,
            },
        );
    }
    // The zero baseline anchors the plot.
    fill_rect(window, 0., width, y_of(0.), y_of(0.) + 1., rgba(0x414650ff));
    if curve.is_empty() {
        return;
    }
    let mut path = PathBuilder::stroke(px(1.5));
    let mut started = false;
    for &(wavelength, value) in curve {
        let into = point(
            bounds.origin.x + px(x_of(wavelength)),
            bounds.origin.y + px(y_of(value)),
        );
        if started {
            path.line_to(into);
        } else {
            path.move_to(into);
            started = true;
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, rgb(0x7fb2f0));
    }
}

impl Render for SpectrumEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let read_only = self.session.read(cx).document().is_read_only();
        let table = self
            .session
            .update(cx, |session, cx| {
                session.spectrum_table(self.entry["id"].as_str().unwrap_or_default(), cx)
            })
            .unwrap_or_else(|| json!({"error":"loading…"}));
        self.load_table(&table, window, cx);
        self.apply_texts(window, cx);
        // One chromaticity swatch replaces the albedo/illuminant
        // pair — the curve's colour, magnitude-free, the same box the
        // Contents thumbnail and the node editor show.
        let chromaticity = self
            .session
            .update(cx, |session, cx| {
                session.spectrum_swatch(
                    self.entry["id"].as_str().unwrap_or_default(),
                    "chromaticity",
                    cx,
                )
            })
            .unwrap_or_else(|| json!({"error":"…"}));

        let draft = self.draft_state();
        let (draft_tables, red_rows, draft_message) = (draft.tables, draft.red_rows, draft.message);
        let changed = self.draft != self.saved;
        let valid = draft_tables.is_some() && changed && !self.storage.is_empty();
        // Review issues 1+2: once the tables are `.array`-backed or longer
        // than one wire edit may carry, the row count cannot change here.
        let rows_fixed = !row_edits_allowed(&self.storage, &self.draft);
        let feedback = super::parameter_form::feedback_text("", &self.session, &self.form, cx);
        let feedback = if !self.message.is_empty() {
            self.message.clone()
        } else if !draft_message.is_empty() {
            draft_message
        } else {
            feedback
        };
        let label = self.entry["name"]
            .as_str()
            .unwrap_or("Spectrum Data")
            .to_owned();
        let curve = match &draft_tables {
            Some((wavelengths, values)) => wavelengths
                .iter()
                .copied()
                .zip(values.iter().copied())
                .collect::<Vec<(f64, f64)>>(),
            None => self.saved_curve.clone(),
        };
        let axes = curve_axes(&curve);
        let curve_for_paint = curve.clone();
        let first = self.page * PAGE_ROWS;
        let last = (first + PAGE_ROWS).min(self.draft.len());
        let pages =
            self.draft.len() / PAGE_ROWS + usize::from(!self.draft.len().is_multiple_of(PAGE_ROWS));

        // The plot: the visible band behind the curve, drawn once the
        // table has loaded. The curve previews the valid draft, or the
        // saved curve while the draft is red. The painter is shared with
        // the Contents row thumbnail. The plot keeps its preferred
        // height while there is room and shrinks toward PLOT_MIN before
        // the table's rows give way (at the default dock
        // height the table showed zero rows).
        let plot = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| paint_curve(bounds, &curve_for_paint, window),
        )
        .absolute()
        .h_full()
        .w_full();

        div()
            .track_focus(&self.focus)
            .size_full()
            .bg(rgb(0x17191e))
            .flex()
            .flex_col()
            .text_color(rgb(0xd4d7dd))
            // The header contains the label and the curve's colour: one
            // chromaticity swatch), apply.
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .child(div().font_weight(FontWeight::BOLD).child(label))
                    .when(!chromaticity.is_null(), |this| {
                        this.child(swatch_box("Colour", &chromaticity))
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("spectrum-add-row")
                            .icon(assets::IconName::Plus)
                            .label("Add Row")
                            .xsmall()
                            .disabled(read_only || rows_fixed)
                            .tooltip("Append a sample row")
                            .on_click(cx.listener(|this, _, window, cx| this.add_row(window, cx))),
                    )
                    .child(
                        Button::new("spectrum-paste")
                            .icon(assets::IconName::ClipboardPaste)
                            .label("Paste Rows")
                            .xsmall()
                            .disabled(read_only || rows_fixed)
                            .tooltip("Append \"wavelength value\" lines from the clipboard")
                            .on_click(cx.listener(|this, _, window, cx| this.paste(window, cx))),
                    )
                    .child(
                        Button::new("spectrum-apply")
                            .icon(assets::IconName::Check)
                            .label("Apply")
                            .xsmall()
                            .disabled(read_only || !valid)
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    ),
            )
            .when(!self.storage.is_empty() && rows_fixed, |this| {
                this.child(
                    div()
                        .px_3()
                        .pb_1()
                        .text_xs()
                        .text_color(rgb(0xd0a86a))
                        .child(
                            "Row add, delete and paste are locked: the sample tables live \
                             in a data asset or exceed the 4096-value edit limit, so edits \
                             rewrite the values in place through set_parameter_values.",
                        ),
                )
            })
            // The curve, with the visible nm range beneath it. The height
            // policy lives on this WRAPPER — the flex item
            // itself — not on an inner div: taffy gives an overflow-visible
            // flex item a content-based automatic minimum, which
            // froze the wrapped 150 px plot and shrank nothing but the
            // table. An explicit min overrides it, so the plot is the item
            // that yields (PLOT_MIN plus the pb_2 here, preferred
            // PLOT_PREFERRED plus the same) while the table keeps its six
            // rows; the inner anchor keeps the canvas inside the padding.
            .child(
                div()
                    .w_full()
                    .px_3()
                    .pb_2()
                    .h(px(PLOT_PREFERRED + 8.))
                    .min_h(px(PLOT_MIN + 8.))
                    .child(div().relative().w_full().h_full().child(plot)),
            )
            .child(
                div()
                    .px_3()
                    .pb_2()
                    .text_xs()
                    .text_color(rgb(0x9098a6))
                    .flex()
                    .justify_between()
                    .child(format!(
                        "{} samples ({} nm – {} nm)",
                        curve.len(),
                        cell_number(axes.0),
                        cell_number(axes.1)
                    ))
                    .child(format!("values up to {}", cell_number(axes.2))),
            )
            // The two-column sample table.
            .child(
                div()
                    .px_3()
                    .flex()
                    .gap_1()
                    .text_xs()
                    .text_color(rgb(0x9098a6))
                    .child(div().w(px(34.)).child("#"))
                    .child(div().w(px(130.)).child("Wavelength (nm)"))
                    .child(div().flex_1().child("Value")),
            )
            // The two-column sample table: at least MIN_TABLE_PX tall
            // It takes whatever height the plot leaves.
            .child(
                div()
                    .id("spectrum-rows")
                    .flex_1()
                    .min_h(px(MIN_TABLE_PX))
                    .overflow_y_scroll()
                    .px_3()
                    .pb_1()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .children(
                        self.draft[first..last]
                            .iter()
                            .zip(self.inputs.chunks(2))
                            .enumerate()
                            .map(|(row, (_, pair))| {
                                let index = first + row;
                                let red = red_rows.contains(&index);
                                let mark_red = move |this: Div| {
                                    if red {
                                        this.text_color(rgb(0xd99a74))
                                    } else {
                                        this
                                    }
                                };
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(mark_red(
                                        div().w(px(34.)).child(format!("{}", index + 1)),
                                    ))
                                    .child(mark_red(
                                        div().w(px(130.)).child(
                                            Input::new(&pair[0])
                                                .xsmall()
                                                .appearance(false)
                                                .rounded_none()
                                                .h(px(23.)),
                                        ),
                                    ))
                                    .child(mark_red(
                                        div().flex_1().child(
                                            Input::new(&pair[1])
                                                .xsmall()
                                                .appearance(false)
                                                .rounded_none()
                                                .h(px(23.)),
                                        ),
                                    ))
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "spectrum-delete-{index}"
                                        )))
                                        .icon(assets::IconName::Minus)
                                        .xsmall()
                                        .disabled(read_only || rows_fixed)
                                        .tooltip("Delete this row")
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.delete_row(index, window, cx)
                                            }),
                                        ),
                                    )
                            }),
                    ),
            )
            // Pages and the message line.
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(self.page > 0, |this| {
                        this.child(
                            Button::new("spectrum-prev-page")
                                .icon(assets::IconName::ChevronLeft)
                                .xsmall()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.page -= 1;
                                    this.apply_texts(window, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        div()
                            .text_color(rgb(0x9098a6))
                            .child(format!("page {} of {pages}", self.page + 1)),
                    )
                    .when(self.page + 1 < pages, |this| {
                        this.child(
                            Button::new("spectrum-next-page")
                                .icon(assets::IconName::ChevronRight)
                                .xsmall()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.page += 1;
                                    this.apply_texts(window, cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when(!feedback.is_empty(), |this| {
                this.child(
                    div()
                        .px_3()
                        .pb_2()
                        .text_xs()
                        .text_color(rgb(0xd99a74))
                        .child(feedback),
                )
            })
    }
}

/// One approximate colour: its box, its role and the human `source`
/// string (`{rgb, source}` or `{error}`).
fn swatch_box(role: &str, answer: &Value) -> Div {
    let error = answer.get("error").and_then(Value::as_str);
    let (color, source) = match error {
        Some(error) => ([1.; 3], error.to_owned()),
        None => {
            let color = answer["rgb"].as_array().map(|channels| {
                [
                    channels.first().and_then(Value::as_f64).unwrap_or(0.) as f32,
                    channels.get(1).and_then(Value::as_f64).unwrap_or(0.) as f32,
                    channels.get(2).and_then(Value::as_f64).unwrap_or(0.) as f32,
                ]
            });
            (
                color.unwrap_or([1., 1., 1.]),
                answer["source"].as_str().unwrap_or_default().to_owned(),
            )
        }
    };
    div()
        .flex()
        .items_center()
        .gap_1()
        .when(error.is_none(), |this| {
            this.child(div().size(px(14.)).flex_shrink_0().rounded_sm().bg(Rgba {
                r: color[0].clamp(0., 1.),
                g: color[1].clamp(0., 1.),
                b: color[2].clamp(0., 1.),
                a: 1.,
            }))
        })
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x9098a6))
                .child(format!("{role} {source}")),
        )
}

impl SpectrumEditor {
    pub fn new(
        session: Entity<Session>,
        entry: &Value,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let target = json!({"kind":"resource","id":entry["id"]});
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let form = format!("spectrum:{}", entry["id"]);
        Self {
            session,
            entry: entry.clone(),
            target,
            form,
            focus: cx.focus_handle(),
            page: 0,
            draft: Vec::new(),
            saved: Vec::new(),
            saved_curve: Vec::new(),
            storage: String::new(),
            loaded: None,
            load_generation: 0,
            inputs: Vec::new(),
            input_key: String::new(),
            message: String::new(),
        }
    }

    /// Load a fetched table payload into the draft if it is new. A clean
    /// draft — or the echo of our own apply — adopts it; a foreign change
    /// under unsaved typing keeps every typed row and says so (review
    /// issue 3: nothing an Apply elsewhere or an undo does may wipe the
    /// table being edited).
    fn load_table(&mut self, table: &Value, window: &mut Window, cx: &mut Context<Self>) {
        let fresh = self.loaded.as_ref().is_none_or(|loaded| loaded != table);
        if !fresh {
            return;
        }
        match rows_from_table(table) {
            Ok(loaded) => match fresh_payload_action(&self.draft, &self.saved, &loaded.rows) {
                FreshPayload::Adopt => {
                    self.message.clear();
                    self.draft = loaded.rows.clone();
                    self.saved = loaded.rows;
                    self.saved_curve = loaded.curve;
                    self.storage = loaded.storage;
                    self.load_generation += 1;
                    let last_page = self.draft.len().saturating_sub(1) / PAGE_ROWS;
                    self.page = self.page.min(last_page);
                    self.loaded = Some(table.clone());
                    self.apply_texts(window, cx);
                }
                FreshPayload::KeepDraft => {
                    // Keep the typing; Apply overwrites the change and
                    // reopening the tab loads the saved version. The
                    // recorded payload keeps this decision to one render
                    // of the change instead of every one.
                    self.loaded = Some(table.clone());
                    self.message = "the spectrum changed elsewhere: apply to \
                                    overwrite it, or close and reopen the tab \
                                    to load the saved version"
                        .into();
                }
            },
            Err(error) => {
                // Keep whatever was loaded; the next revision re-tries.
                self.message = error;
            }
        }
    }

    /// Rebuild the visible page's cell editors when the page or the
    /// loaded table changed.
    fn apply_texts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = format!(
            "{}:{}:{}",
            self.entry["id"], self.page, self.load_generation
        );
        if key == self.input_key {
            return;
        }
        self.input_key = key;
        self.inputs.clear();
        let first = self.page * PAGE_ROWS;
        let last = (first + PAGE_ROWS).min(self.draft.len());
        for row in &self.draft[first..last] {
            for text in [&row.0, &row.1] {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
                let (index, column) = (self.inputs.len() / 2, self.inputs.len() % 2);
                cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let row = this.page * PAGE_ROWS + index;
                        let text = this.inputs[index * 2 + column].read(cx).value().to_string();
                        if column == 0 {
                            this.draft[row].0 = text;
                        } else {
                            this.draft[row].1 = text;
                        }
                        cx.notify();
                    }
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.apply(cx);
                    }
                })
                .detach();
                cx.observe(&input, |_, _, cx| cx.notify()).detach();
                self.inputs.push(input);
            }
        }
    }

    /// The draft's validation: the parsed tables when every row is
    /// editable, the bad rows, and the reviewer's message.
    fn draft_state(&self) -> DraftState {
        let (wavelengths, values) = match parse_rows(&self.draft) {
            Ok(tables) => tables,
            Err(rows) => {
                let first_bad = rows.first().copied().unwrap_or(0);
                return DraftState::rejected(
                    rows,
                    format!("row {}: enter a finite wavelength and value", first_bad + 1),
                );
            }
        };
        match validate_tables(&wavelengths, &values) {
            Ok(()) => DraftState {
                tables: Some((wavelengths, values)),
                red_rows: Vec::new(),
                message: String::new(),
            },
            Err(message) => DraftState {
                tables: None,
                red_rows: Vec::new(),
                message,
            },
        }
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !row_edits_allowed(&self.storage, &self.draft) {
            return;
        }
        let wavelength = match self.draft.last() {
            None => 400.,
            Some(row) => {
                let last = row.0.parse::<f64>().ok().unwrap_or(400.);
                let step = if self.draft.len() >= 2 {
                    (last
                        - self.draft[self.draft.len() - 2]
                            .0
                            .parse::<f64>()
                            .unwrap_or(390.))
                    .abs()
                } else {
                    10.
                }
                .clamp(1., 100.);
                last + step
            }
        };
        let value = self
            .draft
            .last()
            .map(|row| row.1.clone())
            .unwrap_or_else(|| "1".into());
        self.draft.push((cell_number(wavelength), value));
        self.page = self.draft.len().saturating_sub(1) / PAGE_ROWS;
        // The row count changed on the same page: rebuild its inputs.
        self.input_key.clear();
        self.apply_texts(window, cx);
        cx.notify();
    }

    fn delete_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.draft.len() {
            return;
        }
        self.draft.remove(index);
        let last_page = self.draft.len().saturating_sub(1) / PAGE_ROWS;
        self.page = self.page.min(last_page);
        self.input_key.clear();
        self.apply_texts(window, cx);
        cx.notify();
    }

    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !row_edits_allowed(&self.storage, &self.draft) {
            self.message = "the row count is fixed here; paste onto a shorter inline table".into();
            cx.notify();
            return;
        }
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        match paste_rows(&text) {
            Ok(mut rows) => {
                // Length changes ride the single transaction apply, whose
                // per-edit value limit bounds the row count: paste only
                // as many rows as fit under it.
                let room = WIRE_WINDOW_VALUES - self.draft.len();
                if rows.len() > room {
                    self.message = format!(
                        "pasted {} of {} rows — the editor holds at most {} rows",
                        room,
                        rows.len(),
                        WIRE_WINDOW_VALUES
                    );
                    rows.truncate(room);
                } else {
                    self.message.clear();
                }
                self.draft.extend(rows);
                self.page = self.draft.len().saturating_sub(1) / PAGE_ROWS;
                self.input_key.clear();
                self.apply_texts(window, cx);
            }
            Err(error) => self.message = error,
        }
        cx.notify();
    }

    /// Write the draft back (review issues 1+2): inline tables under the
    /// per-edit limit go as one `project.apply` transaction — a single
    /// Undo step; `.array`-backed or over-cap tables keep their fixed
    /// length and rewrite per-table through chunked
    /// `project.set_parameter_values` windows.
    fn apply(&mut self, cx: &mut Context<Self>) {
        let Some((wavelengths, values)) = self.draft_state().tables else {
            return;
        };
        let route = write_route(&self.storage, &self.draft);
        let form = self.form.clone();
        self.session.update(cx, |session, cx| {
            let project = session.project_value();
            if !project.is_object() || session.document().is_read_only() {
                return;
            }
            match route {
                WriteRoute::ApplyBothTables => {
                    session.edit_feedback.submitted(&form);
                    let edits = set_parameter_edits(&self.target, &wavelengths, &values);
                    session.send(
                        Command::Apply {
                            project,
                            edits,
                            follow: Follow::Nothing,
                            form: Some(form),
                        },
                        cx,
                    );
                }
                WriteRoute::ValueWindows => {
                    // One job per table, run in order: the second needs the
                    // revision the first published.
                    let target = self.target.clone();
                    session.edit_feedback.submitted(&form);
                    let steps = [("wavelengths", wavelengths), ("values", values)]
                        .into_iter()
                        .map(|(name, floats)| value_window_params(&project, &target, name, &floats))
                        .collect();
                    session.send(
                        Command::SubmitSequence {
                            method: protocol::Method::ProjectSetParameterValues,
                            steps,
                            form,
                        },
                        cx,
                    );
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    // Specific imports, not `use super::*`: the glob would chain through
    // gpui-kit's exports and shadow `#[test]` itself.
    use super::{
        MAX_SAMPLES, Row, WIRE_WINDOW_VALUES, cell_number, curve_axes, fresh_payload_action,
        parse_rows, paste_rows, row_edits_allowed, rows_from_table, set_parameter_edits,
        validate_tables, value_window_params, visible_bands, write_route,
    };
    use serde_json::json;

    #[test]
    fn rows_parse_into_two_tables_and_name_bad_rows() {
        let rows = vec![
            ("400".into(), "1".into()),
            ("nan".into(), "0.5".into()),
            ("500".into(), "two".into()),
        ];
        let bad = parse_rows(&rows).unwrap_err();
        assert_eq!(bad, vec![1, 2]);
        let one = parse_rows(&[("400".into(), "1".into())]).unwrap();
        assert_eq!(one, (vec![400.], vec![1.]));
    }

    #[test]
    fn tables_validate_ascending_positive_pairs_within_the_cap() {
        assert!(validate_tables(&[400., 500., 600.], &[1., 2., 3.]).is_ok());
        // Equal neighbours are valid in the project model; only a
        // descending pair is rejected.
        assert!(validate_tables(&[400., 400., 500.], &[1., 2., 3.]).is_ok());
        assert_eq!(
            validate_tables(&[400., 500., 450.], &[1., 2., 3.]).unwrap_err(),
            "row 2: wavelengths must not descend"
        );
        assert_eq!(
            validate_tables(&[0., 400.], &[1., 2.]).unwrap_err(),
            "row 0: wavelengths must be positive"
        );
        assert_eq!(
            validate_tables(&[], &[]).unwrap_err(),
            "the spectrum needs at least one sample"
        );
        let many: Vec<f64> = (0..=MAX_SAMPLES).map(|i| 300. + i as f64).collect();
        let ones = vec![1.; many.len()];
        assert!(
            validate_tables(&many, &ones)
                .unwrap_err()
                .contains("too many samples"),
            "{} rows",
            many.len()
        );
        assert!(validate_tables(&[400.], &[1.]).is_ok());
    }

    #[test]
    fn clipboard_text_pastes_tab_or_space_separated_pairs() {
        assert_eq!(
            paste_rows("400\t1\n500  2\n\n600 \t 3\n").unwrap(),
            vec![
                ("400".into(), "1".into()),
                ("500".into(), "2".into()),
                ("600".into(), "3".into()),
            ]
        );
        assert!(
            paste_rows("400 1\nbroken line\n")
                .unwrap_err()
                .contains("line 2")
        );
        assert!(paste_rows("   \n").unwrap().is_empty());
    }

    #[test]
    fn inline_edits_replace_both_tables_in_one_apply() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let edits = set_parameter_edits(&target, &[400., 700.], &[1., 0.5]);
        assert_eq!(edits.len(), 2);
        for (edit, name) in edits.iter().zip(["wavelengths", "values"]) {
            assert_eq!(edit["kind"], "set_parameter");
            assert_eq!(edit["target"], target);
            assert_eq!(edit["name"], name);
            assert_eq!(edit["value_type"], "float");
            assert_eq!(edit["value"]["values"]["kind"], "floats");
            // The wire type itself must accept what the editor sends.
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
        assert_eq!(edits[0]["value"]["values"]["value"], json!([400., 700.]));
        assert_eq!(edits[1]["value"]["values"]["value"], json!([1., 0.5]));
    }

    #[test]
    fn big_tables_rewrite_through_chunked_windows_under_the_wire_caps() {
        let project = json!({"project_id":1});
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        // Review issue 1: the .array tables the editor opens always hold
        // more than 4096 values, and one whole-table window breaks the
        // wire's per-window limit — the rewrite must arrive chunked and
        // contiguous, however long the table is.
        let many: Vec<f64> = (0..5000).map(|index| index as f64).collect();
        let params = value_window_params(&project, &target, "values", &many);
        let windows = params["windows"].as_array().cloned().unwrap();
        assert_eq!(windows.len(), 2, "{params}");
        assert_eq!(windows[0]["offset"], 0);
        assert_eq!(
            windows[0]["values"]["value"].as_array().map(Vec::len),
            Some(WIRE_WINDOW_VALUES)
        );
        assert_eq!(windows[1]["offset"], WIRE_WINDOW_VALUES);
        assert_eq!(
            windows[1]["values"]["value"].as_array().map(Vec::len),
            Some(5000 - WIRE_WINDOW_VALUES)
        );
        assert_eq!(
            windows[0]["values"]["value"].as_array().unwrap()[WIRE_WINDOW_VALUES - 1],
            (WIRE_WINDOW_VALUES - 1) as f64
        );
    }

    #[test]
    fn write_routes_pick_the_one_undo_step_where_the_wire_allows_it() {
        use super::WriteRoute;
        let rows = |count: usize| -> Vec<Row> {
            (0..count)
                .map(|index| (format!("{}", 300 + index), "1".into()))
                .collect()
        };
        // Inline tables up to the cap apply as one transaction — the
        // single undo step of set_parameter × 2 in one project.apply.
        assert_eq!(write_route("floats", &rows(1)), WriteRoute::ApplyBothTables);
        assert_eq!(
            write_route("floats", &rows(WIRE_WINDOW_VALUES)),
            WriteRoute::ApplyBothTables
        );
        // Review issues 1+2: a set_parameter edit cannot carry .array
        // storage or more than 4096 values, so the same fixed length
        // rewrites through chunked value windows instead.
        assert_eq!(write_route("array", &rows(1)), WriteRoute::ValueWindows);
        assert_eq!(
            write_route("floats", &rows(WIRE_WINDOW_VALUES + 1)),
            WriteRoute::ValueWindows
        );
        // Row add/delete/paste need that length-changing apply: locked
        // for .array tables and from the cap up.
        assert!(row_edits_allowed("floats", &rows(1)));
        assert!(!row_edits_allowed("array", &rows(1)));
        assert!(!row_edits_allowed("floats", &rows(WIRE_WINDOW_VALUES)));
        assert!(!row_edits_allowed("floats", &rows(WIRE_WINDOW_VALUES + 1)));
    }

    #[test]
    fn a_dirty_draft_survives_a_foreign_revision() {
        use super::FreshPayload::*;
        let saved = vec![("400".to_string(), "1".to_string())];
        // A clean draft adopts whatever the project answers.
        assert_eq!(fresh_payload_action(&[], &[], &saved), Adopt);
        // The payload of our own apply echoes the draft — even spelled
        // less canonically in the cells — and adoption settles the
        // applied state without touching the typing.
        let draft = vec![("400".to_string(), "2".to_string())];
        let verbose = vec![("400.0".to_string(), "2.0".to_string())];
        assert_eq!(fresh_payload_action(&draft, &saved, &draft), Adopt);
        assert_eq!(fresh_payload_action(&verbose, &saved, &draft), Adopt);
        // A foreign change under local typing keeps every unsaved row.
        let foreign = vec![("500".to_string(), "9".to_string())];
        assert_eq!(fresh_payload_action(&draft, &saved, &foreign), KeepDraft);
    }

    #[test]
    fn fetched_tables_become_rows_and_curves() {
        let table = json!({"wavelengths":[400.,700.],"values":[1.,0.5],
                           "storage":"floats"});
        let loaded = rows_from_table(&table).unwrap();
        assert_eq!(
            loaded.rows,
            vec![
                ("400".to_string(), "1".to_string()),
                ("700".to_string(), "0.5".to_string())
            ]
        );
        assert_eq!(loaded.curve, vec![(400., 1.), (700., 0.5)]);
        assert_eq!(loaded.storage, "floats");
        assert_eq!(
            rows_from_table(&json!({"error":"missing"})).unwrap_err(),
            "missing"
        );
    }

    #[test]
    fn curve_axes_pad_the_data_and_fall_back_when_degenerate() {
        assert_eq!(curve_axes(&[(400., 1.), (700., 2.)]), (400., 700., 2.));
        // One sample still spans a visible x range.
        assert!(curve_axes(&[(500., 1.)]).0 < 500. && curve_axes(&[(500., 1.)]).1 > 500.);
        // A flat zero curve keeps a visible y range.
        assert_eq!(curve_axes(&[(400., 0.)]).2, 1.);
        // Negative values extend below zero: the y max keeps the headroom.
        assert_eq!(curve_axes(&[(400., -0.5)]).2, 1.);
    }

    #[test]
    fn visible_bands_cover_the_visible_wavelengths() {
        let bands = visible_bands();
        assert_eq!(bands.first().unwrap().0, 380.);
        assert_eq!(bands.last().unwrap().0, 780.);
        // 550 nm is green: its blue channel trails its green.
        let green = bands.iter().find(|band| band.0 == 550.).unwrap().1;
        assert!(green[1] > green[2], "{green:?}");
    }

    #[test]
    fn cell_numbers_trim_trailing_zeros() {
        assert_eq!(cell_number(400.), "400");
        assert_eq!(cell_number(0.25), "0.25");
        assert_eq!(cell_number(-0.), "0");
    }
}
