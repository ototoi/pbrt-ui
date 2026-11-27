//! The typed parameter rows of one target: the
//! table's rows — declared, or the default shown greyed — then the
//! declared parameters the table does not know. Pure so the shapes can be
//! tested without a window; `parameter_field` renders them, in the
//! Inspector and in the node editor alike.
use serde_json::{Value, json};

/// How one row is shown and edited.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    /// One float or integer (text keeps integers above 2^53 exact).
    Number {
        integer: bool,
        value: String,
    },
    /// An rgb triple.
    Rgb([f64; 3]),
    /// A vector, point or normal: x / y / z.
    Vector([f64; 3]),
    /// A builtin spectrum name.
    Spectrum(String),
    /// A reference to a SpectrumData resource (`Resources{spectrum_data}`):
    /// its curve is the value, its editor edits it.
    SpectrumData(String),
    Bool(bool),
    /// One plain string.
    Text(String),
    /// Connected to a texture.
    Texture(String),
    /// Mix's material list (read-only links).
    Materials(Vec<String>),
    /// A managed file (link to the asset).
    Asset(String),
    /// Long arrays, unusual types and truncated previews keep the JSON form.
    Json,
    /// An undeclared row these fields cannot declare (files take an asset,
    /// point lists and spectra need data): only its hint shows.
    Unset,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub value_type: String,
    /// The owning component's implementation, for layouts that depend on
    /// it (a mesh's `indices` groups by its element width); empty when
    /// the page does not say.
    pub implementation: String,
    /// False for a table row shown with its default (or empty).
    pub declared: bool,
    /// The table knows this parameter.
    pub known: bool,
    pub required: bool,
    /// The texture lineage this row accepts ("float" / "spectrum"), from
    /// the table; None when it takes no texture.
    pub texture: Option<String>,
    /// The raw texture kind the table declares ("spectrum_albedo",
    /// "spectrum_unbounded", …), before it collapses to a lineage; the
    /// colour box and curve sample tell their roles apart by it.
    pub texture_kind: Option<String>,
    /// The table's value types for this row (`["spectrum","rgb"]`),
    /// which spectrum dialog tabs it allows; empty for a row the table
    /// does not know.
    pub value_types: Vec<String>,
    /// A managed-file row (the table's file role): it takes an asset.
    pub file: bool,
    /// The Inspector component the table puts this row in ("mapping");
    /// None for the body.
    pub group: Option<String>,
    /// (parameter, values): r4 reads the row only while each holds.
    pub when: Vec<(String, Vec<String>)>,
    /// The values the table allows for this string row; empty = free text.
    pub choices: Vec<String>,
    /// Common values the table offers for a free-text row.
    pub suggestions: Vec<String>,
    pub field: Field,
    /// Why r4 reads none of this conductor's eta/k — a declared
    /// reflectance replaced it, or r4 refuses the both-declared pair.
    /// None on every other row.
    pub ignored: Option<&'static str>,
    /// What r4 uses while undeclared and the row has no constant default
    /// ("follows radius", "illuminant", …); empty otherwise.
    pub hint: String,
    /// The declared preview, for the JSON form and the color space.
    pub parameter: Value,
}

fn float(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value["value"].as_f64())
}

/// The three-float geometric types the document accepts (r4 also reads
/// the bare vector / normal).
const GEOMETRIC: &[&str] = &["vector", "vector3", "point3", "normal", "normal3"];

/// The field for a declared parameter preview (or a synthesized default).
/// `tuple`: the table says the row is one geometric tuple, not an array
/// (a mesh's N) that merely holds three values.
fn field(parameter: &Value, tuple: bool) -> Field {
    let values = &parameter["values"];
    let list = values["value"].as_array().cloned().unwrap_or_default();
    if parameter["preview_truncated"] == true {
        return Field::Json;
    }
    let value_type = parameter["value_type"].as_str().unwrap_or_default();
    match (values["kind"].as_str().unwrap_or_default(), list.len()) {
        ("floats", 3) if value_type == "rgb" => {
            match (float(&list[0]), float(&list[1]), float(&list[2])) {
                (Some(r), Some(g), Some(b)) => Field::Rgb([r, g, b]),
                _ => Field::Json,
            }
        }
        ("floats", 3) if tuple && GEOMETRIC.contains(&value_type) => {
            match (float(&list[0]), float(&list[1]), float(&list[2])) {
                (Some(x), Some(y), Some(z)) => Field::Vector([x, y, z]),
                _ => Field::Json,
            }
        }
        ("floats", 1) if matches!(value_type, "float" | "blackbody") => match float(&list[0]) {
            Some(value) => Field::Number {
                integer: false,
                value: value.to_string(),
            },
            None => Field::Json,
        },
        // Declared integers come as decimal strings, the table's as numbers.
        ("integers", 1) => Field::Number {
            integer: true,
            value: list[0]
                .as_str()
                .map(str::to_owned)
                .or_else(|| list[0].as_i64().map(|n| n.to_string()))
                .unwrap_or_default(),
        },
        ("booleans", 1) => Field::Bool(list[0] == true),
        ("strings", 1) => Field::Text(list[0].as_str().unwrap_or_default().to_owned()),
        ("builtin_spectra", 1) => Field::Spectrum(list[0].as_str().unwrap_or_default().to_owned()),
        ("assets", _) => match list.first().and_then(Value::as_str) {
            Some(id) => Field::Asset(id.to_owned()),
            None => Field::Json,
        },
        _ if values["kind"] == "resources" => {
            let ids = values["ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            match (values["resource_kind"].as_str(), ids.as_slice()) {
                (Some("texture"), [id]) => Field::Texture(id.clone()),
                (Some("material"), _) => Field::Materials(ids),
                (Some("spectrum_data"), [id]) => Field::SpectrumData(id.clone()),
                _ => Field::Json,
            }
        }
        _ => Field::Json,
    }
}

/// An undeclared table row's field: its constant default, else an empty
/// field of the row's first simple type.
fn default_field(spec: &Value) -> (Field, String) {
    let first = spec["value_types"][0]
        .as_str()
        .unwrap_or("float")
        .to_owned();
    // A file is an asset reference, never typed text.
    if spec["role"] == "file" {
        return (Field::Unset, first);
    }
    let default = &spec["default"];
    if default.is_object() {
        let values = default["values"].as_array().cloned().unwrap_or_default();
        let value_type = default["value_type"].as_str().unwrap_or_default();
        let kind = match value_type {
            "integer" => "integers",
            "bool" => "booleans",
            "string" => "strings",
            "spectrum" if values.iter().all(Value::is_string) => "builtin_spectra",
            _ => "floats",
        };
        let preview = json!({"value_type":value_type,"values":{"kind":kind,"value":values}});
        // A tuple the fields cannot type shows as its hint.
        // The table gives only single tuples a three-value default.
        let field = field(&preview, values.len() == 3);
        if field == Field::Json {
            return (Field::Unset, value_type.to_owned());
        }
        return (field, value_type.to_owned());
    }
    let accepts = |kind: &str| {
        spec["value_types"]
            .as_array()
            .is_some_and(|types| types.iter().any(|t| t == kind))
    };
    // Colors take zeros greyed out until edited; the hint says "not set".
    if accepts("rgb") {
        return (Field::Rgb([0.0; 3]), "rgb".into());
    }
    let value_type = first;
    let empty = match value_type.as_str() {
        "float" | "blackbody" => Field::Number {
            integer: false,
            value: String::new(),
        },
        "integer" => Field::Number {
            integer: true,
            value: String::new(),
        },
        "bool" => Field::Bool(false),
        "string" => Field::Text(String::new()),
        _ => Field::Unset,
    };
    (empty, value_type)
}

/// A table row's texture lineage: float, or spectrum for the spectrum
/// interpretation classes (albedo, unbounded, illuminant).
fn lineage(spec: &Value) -> Option<String> {
    spec["texture"]["kind"]
        .as_str()
        .map(|kind| if kind == "float" { "float" } else { "spectrum" }.to_owned())
}

fn hint(spec: &Value) -> String {
    match spec["default_kind"].as_str() {
        Some("depends") => format!(
            "follows {}",
            spec["default_depends"]["parameter"]
                .as_str()
                .unwrap_or("another")
        ),
        Some("illuminant") => "color space illuminant".into(),
        Some("medium_preset") => "medium preset".into(),
        _ if spec["role"] == "file" => "file: not set".into(),
        Some("none") if spec["required"] == true => "required".into(),
        Some("none") => "not set".into(),
        _ => String::new(),
    }
}

/// A row the table defaults to three values is one tuple, not an array.
fn single_tuple(spec: &Value) -> bool {
    spec["default"]["values"]
        .as_array()
        .is_some_and(|values| values.len() == 3)
}

/// The read an eta/k row's r4 implicit default keeps, when the
/// metal reads its reflectance instead (`unused: reflectance`).
pub const UNUSED_REFLECTANCE: &str = "unused: reflectance";
/// R4 refuses a conductor declaring a reflectance beside eta/k
/// (r4 materials/conductor.rs); the Metal colour row and both rows'
/// hints carry this warning.
pub const REFLECTANCE_WITH_ETA_K: &str = "r4 rejects reflectance together with eta/k";

/// How a parameter row shows its spectrum-ish value: the one
/// colour box of the dialog for every spectrum-capable row — an
/// unbounded `eta`/`k` included, its colour being the curve's
/// chromaticity — or, for every other row, the typed field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectrumView {
    /// Not spectrum-capable (or the table does not know the row): the
    /// row keeps its typed field.
    Typed,
    /// One approximate colour box; clicking it opens the colour dialog.
    Colour,
}

/// Which of the colour dialog's tabs the schema allows for a row
/// (reflectance takes no blackbody, a light takes all three).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpectrumTabs {
    pub rgb: bool,
    pub blackbody: bool,
    pub named: bool,
    pub spectrum: bool,
}

/// One tab of the colour dialog: the way a value is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectrumTab {
    Rgb,
    Blackbody,
    /// A built-in spectrum picked by name.
    Named,
    /// A definition the project holds: a Spectrum Data resource, an `.spd`
    /// file or inline wavelength/value samples.
    Spectrum,
}

/// The schema's value types of `spec` (empty when the table does not
/// declare the row).
fn spec_value_types(spec: &Value) -> Vec<String> {
    spec["value_types"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|kind| kind.as_str().map(str::to_owned))
        .collect()
}

/// A spectrum-capable row: the table knows it and its value types hold
/// `spectrum`, so spectral values of any shape may sit in it.
fn spectrum_sources(row: &Row) -> Option<&[String]> {
    (row.known && row.value_types.iter().any(|kind| kind == "spectrum"))
        .then_some(row.value_types.as_slice())
}

/// How a row shows its spectrum-ish value (see `SpectrumView`).
pub fn spectrum_view(row: &Row) -> SpectrumView {
    // A number (a dielectric's `float eta`, declared or as the greyed
    // default) keeps its number field: a colour box would hide the value.
    if row.value_type == "float" {
        return SpectrumView::Typed;
    }
    match spectrum_sources(row) {
        Some(_) => SpectrumView::Colour,
        None => SpectrumView::Typed,
    }
}

/// The role the row's swatch asks `project.spectrum_color` for: a
/// reflectance keeps the albedo read (the colour the surface would
/// have); a light's colour row and an unbounded eta/k show the
/// chromaticity, magnitude-free (the light's brightness stays in its
/// numbers and the dialog's intensity). None leaves the choice to the
/// core's schema-derived role.
pub fn row_role(row: &Row) -> Option<String> {
    match row.texture_kind.as_deref() {
        Some("spectrum_albedo") => Some("albedo".into()),
        Some("spectrum_unbounded" | "spectrum_illuminant") => Some("chromaticity".into()),
        // A light's colour row the table gives no texture kind: the
        // blackbody heuristic the dialog keys on recognizes it.
        None if spectrum_tabs(row).blackbody => Some("chromaticity".into()),
        _ => None,
    }
}

/// The short muted label beside a spectrum row's colour
/// box, naming what the value is — the resource's name for a Spectrum
/// Data reference (`Au.eta`), the builtin's name for a named spectrum
/// (`metal-Cu-eta`), `5500 K` for a blackbody. Rgb carries no label (a
/// colour needs no name), and neither does an unset row or a shape with
/// no name of its own (inline samples, a texture link); the render
/// truncates the label and carries the full text in its tooltip.
pub fn value_label(row: &Row, items: &Value) -> Option<String> {
    if !row.declared {
        return None;
    }
    match &row.field {
        Field::SpectrumData(id) => Some(super::components::item_name(items, id)),
        Field::Spectrum(name) => Some(name.clone()),
        Field::Number { value, .. } if row.value_type == "blackbody" => Some(format!("{value} K")),
        _ => None,
    }
}

/// The tabs the colour dialog offers `row`: one per input shape the
/// schema's value types allow. All false for a row that is not
/// spectrum-capable.
pub fn spectrum_tabs(row: &Row) -> SpectrumTabs {
    let allows =
        |kind: &str| spectrum_sources(row).is_some_and(|types| types.iter().any(|t| t == kind));
    SpectrumTabs {
        rgb: allows("rgb"),
        blackbody: allows("blackbody"),
        named: allows("spectrum"),
        spectrum: allows("spectrum"),
    }
}

/// The `project.apply` edits that set `row` to `values` typed
/// `value_type`. A cross-type change replaces a declared declaration
/// first — the core refuses to silently change a declaration's type — so
/// both edits ride one transaction and one Undo step returns the old
/// value. An explicit colour space survives only an rgb-to-rgb update:
/// it keys colour values, not temperatures or curves.
pub fn colour_edits(target: &Value, row: &Row, value_type: &str, values: Value) -> Vec<Value> {
    let mut edits = Vec::new();
    if row.declared && row.value_type != value_type {
        edits.push(
            json!({"kind":"set_parameter","target":target,"name":row.name,
                          "value_type":row.value_type,"value":null}),
        );
    }
    let color_space = if row.declared && row.value_type == "rgb" && value_type == "rgb" {
        row.parameter["color_space"]["value"].clone()
    } else {
        Value::Null
    };
    edits.push(
        json!({"kind":"set_parameter","target":target,"name":row.name,
                      "value_type":value_type,
                      "value":{"name":row.name,"value_type":value_type,
                               "color_space":color_space,"values":values}}),
    );
    edits
}

/// (items 2–3): the family of built-in named spectra a
/// parameter row's role should pick from — a conductor's eta/k want
/// the metal curves, a dielectric's eta the glasses, a light's colour
/// row the standard illuminants. The family names the Named tab's
/// default and the list it shows first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedFamily {
    /// `metal-*-eta` (a conductor's eta), default `metal-Cu-eta`.
    MetalEta,
    /// `metal-*-k` (a conductor's k), default `metal-Cu-k`.
    MetalK,
    /// Names starting `glass-` (a dielectric's eta), default `glass-BK7`.
    Glass,
    /// `stdillum-*` or `illum-*` (a light's colour), default
    /// `stdillum-D65`.
    Illuminant,
}

impl NamedFamily {
    /// Whether a built-in name belongs to this family.
    pub fn contains(&self, name: &str) -> bool {
        match self {
            Self::MetalEta => name.starts_with("metal-") && name.ends_with("-eta"),
            Self::MetalK => name.starts_with("metal-") && name.ends_with("-k"),
            Self::Glass => name.starts_with("glass-"),
            Self::Illuminant => name.starts_with("stdillum-") || name.starts_with("illum-"),
        }
    }

    /// The name a row of this family defaults to: its canonical curve
    /// when the table has it, else the family's first member. `None`
    /// when the table holds none of the family — the caller falls back
    /// to today's default.
    pub fn default_name<'a>(&self, builtins: &'a [String]) -> Option<&'a String> {
        let canonical = match self {
            Self::MetalEta => "metal-Cu-eta",
            Self::MetalK => "metal-Cu-k",
            Self::Glass => "glass-BK7",
            Self::Illuminant => "stdillum-D65",
        };
        builtins
            .iter()
            .find(|name| name.as_str() == canonical)
            .or_else(|| builtins.iter().find(|name| self.contains(name)))
    }

    /// The short name the "show only" toggle offers the family by.
    pub fn label(&self) -> &'static str {
        match self {
            Self::MetalEta | Self::MetalK => "metal",
            Self::Glass => "glass",
            Self::Illuminant => "light",
        }
    }
}

/// (items 2–3): the family of named spectra `row` belongs to. The
/// light colour rows (the blackbody-capable ones — only the lights' I
/// and L ever allow that tab) are illuminants; a (coated)conductor's
/// eta/k are metals; a (thin)dielectric's eta and a
/// coateddiffuse/coatedconductor interface.eta are glasses. The row's
/// implementation comes from the owning page; a page that cannot name
/// one falls back to the name alone (eta/k are the metals' operands).
/// Anything else fits no family: the list stays full and the default
/// today's.
pub fn named_family(row: &Row) -> Option<NamedFamily> {
    if spectrum_tabs(row).blackbody {
        return Some(NamedFamily::Illuminant);
    }
    let (implementation, parameter) = (row.implementation.as_str(), row.name.as_str());
    match (implementation, parameter) {
        ("conductor" | "coatedconductor", "eta" | "conductor.eta") => Some(NamedFamily::MetalEta),
        ("conductor" | "coatedconductor", "k" | "conductor.k") => Some(NamedFamily::MetalK),
        ("dielectric" | "thindielectric", "eta") => Some(NamedFamily::Glass),
        ("coateddiffuse" | "coatedconductor", "interface.eta") => Some(NamedFamily::Glass),
        // A value-node page that cannot know its consuming material:
        // the name alone names the metals' operands.
        ("", "eta") => Some(NamedFamily::MetalEta),
        ("", "k") => Some(NamedFamily::MetalK),
        _ => None,
    }
}

/// The tab-switch edits — the value replaced with the tab type's
/// default (plan: rgb `[1 1 1]`, blackbody `6500 K`, the Spectrum
/// tab's first pick). The Spectrum default is the project's first
/// SpectrumData resource (sorted by name, the way every Contents choice
/// lists), else the first builtin spectrum `builtins` names; with
/// neither at hand the tab opens without an edit (it still lists the
/// import action). The Named default is the row's own family's curve;
/// a row without a family keeps the neutral white — stdillum-D65, else the first name.
pub fn tab_default_edits(
    target: &Value,
    row: &Row,
    tab: SpectrumTab,
    items: &Value,
    builtins: &[String],
) -> Vec<Value> {
    let (value_type, values) = match tab {
        SpectrumTab::Rgb => ("rgb", json!({"kind":"floats","value":[1., 1., 1.]})),
        SpectrumTab::Blackbody => ("blackbody", json!({"kind":"floats","value":[6500.]})),
        // D65 is the neutral white of the standard illuminants; without it
        // the first name.
        SpectrumTab::Named => match named_family(row)
            .and_then(|family| family.default_name(builtins))
            .or_else(|| {
                builtins
                    .iter()
                    .find(|name| name.as_str() == "stdillum-D65")
                    .or_else(|| builtins.first())
            }) {
            Some(name) => ("spectrum", json!({"kind":"builtin_spectra","value":[name]})),
            None => return Vec::new(),
        },
        SpectrumTab::Spectrum => match first_spectrum_data(items) {
            Some(id) => (
                "spectrum",
                json!({"kind":"resources","value":{"kind":"spectrum_data","ids":[id]}}),
            ),
            None => return Vec::new(),
        },
    };
    colour_edits(target, row, value_type, values)
}

/// The Spectrum Data tab's default pick: the project's first SpectrumData
/// resource by name.
fn first_spectrum_data(items: &Value) -> Option<String> {
    let mut choice: Option<(String, String)> = None;
    for (id, item) in items.as_object().into_iter().flatten() {
        if item["resource_kind"] != "spectrum_data" {
            continue;
        }
        let name = item["name"].as_str().unwrap_or(id).to_owned();
        if choice.as_ref().is_none_or(|current| name < current.0) {
            choice = Some((name, id.clone()));
        }
    }
    choice.map(|(_, id)| id)
}

/// The light's `scale` row the colour dialog shows beside a
/// blackbody temperature: a page whose colour row
/// allows blackbody (only the lights' I and L do) that also declares a
/// plain constant `scale`. A mix texture's `scale` operand is a texture
/// input with no blackbody tab anywhere, so it never shows.
pub fn dialog_scale(rows: &[Row]) -> Option<Row> {
    if !rows.iter().any(|row| spectrum_tabs(row).blackbody) {
        return None;
    }
    rows.iter()
        .find(|row| {
            row.known
                && row.name == "scale"
                && row.texture.is_none()
                && row.value_types.iter().any(|kind| kind == "float")
        })
        .cloned()
}

/// The tab the row's current value belongs to — the declared
/// value's spectral kind, or the greyed default's. The dialog opens on
/// it; switching away from it replaces the value (one Undo step).
pub fn current_tab(row: &Row) -> SpectrumTab {
    if row.declared {
        let kind = row.parameter["values"]["kind"].as_str();
        if matches!(kind, Some("builtin_spectra")) {
            return SpectrumTab::Named;
        }
        if matches!(kind, Some("resources" | "assets")) {
            return SpectrumTab::Spectrum;
        }
        return match row.value_type.as_str() {
            "rgb" => SpectrumTab::Rgb,
            "blackbody" => SpectrumTab::Blackbody,
            _ => SpectrumTab::Spectrum,
        };
    }
    match (&row.field, row.value_type.as_str()) {
        (Field::Spectrum(_), _) => SpectrumTab::Named,
        (Field::Number { .. }, "blackbody") => SpectrumTab::Blackbody,
        // A greyed scalar on a schema with no rgb to hold the wheel
        // The Spectrum tab — the scalar field's
        // home, and the one the tab rule offers at all.
        (Field::Number { .. }, "float") if !spectrum_tabs(row).rgb => SpectrumTab::Spectrum,
        _ => SpectrumTab::Rgb,
    }
}

fn group(spec: &Value) -> Option<String> {
    spec["group"].as_str().map(str::to_owned)
}

fn strings(values: &Value) -> Vec<String> {
    values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

fn when(spec: &Value) -> Vec<(String, Vec<String>)> {
    spec["when"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|condition| {
            let parameter = condition["parameter"].as_str()?.to_owned();
            Some((parameter, strings(&condition["values"])))
        })
        .collect()
}

/// Whether r4 reads `row` under the current values of `rows` (declared,
/// else the table's default). A declared row always shows, so hiding
/// never hides a value the document carries; neither does a value outside
/// the table's choices (an import or the CLI), which says nothing.
pub fn in_use(rows: &[Row], row: &Row) -> bool {
    row.declared
        || row.when.iter().all(|(parameter, values)| {
            let Some(other) = rows.iter().find(|other| other.name == *parameter) else {
                return false;
            };
            let current = match &other.field {
                Field::Text(text) => text.as_str(),
                Field::Number { value, .. } => value.as_str(),
                _ => return false,
            };
            values.iter().any(|value| value == current)
                || !other.choices.is_empty() && !other.choices.iter().any(|c| c == current)
        })
}

/// A choice or suggestion as a menu shows it: the empty value is "none".
pub fn choice_label(value: &str) -> String {
    if value.is_empty() {
        "(none)".into()
    } else {
        value.into()
    }
}

/// The rows for one page `{parameters, schema, implementation}`; `schema`
/// is the table's rows, or null for an implementation without a table.
/// `implementation` (the owning component's) rides along for layouts
/// that depend on it.
pub fn rows(page: &Value) -> Vec<Row> {
    let declared = page["parameters"].as_array().cloned().unwrap_or_default();
    let schema = page["schema"].as_array().cloned().unwrap_or_default();
    let matches = |spec: &Value, name: &str| {
        spec["name"] == name
            || spec["aliases"]
                .as_array()
                .is_some_and(|aliases| aliases.iter().any(|alias| alias == name))
    };
    let implementation = page["implementation"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut rows = Vec::new();
    for spec in &schema {
        let required = spec["required"] == true;
        match declared
            .iter()
            .find(|p| matches(spec, p["name"].as_str().unwrap_or_default()))
        {
            Some(parameter) => rows.push(Row {
                name: parameter["name"].as_str().unwrap_or_default().to_owned(),
                value_type: parameter["value_type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                implementation: implementation.clone(),
                declared: true,
                known: true,
                required,
                texture: lineage(spec),
                texture_kind: spec["texture"]["kind"].as_str().map(str::to_owned),
                value_types: spec_value_types(spec),
                file: spec["role"] == "file",
                group: group(spec),
                when: when(spec),
                choices: strings(&spec["choices"]),
                suggestions: strings(&spec["suggestions"]),
                field: field(parameter, single_tuple(spec)),
                ignored: None,
                hint: String::new(),
                parameter: parameter.clone(),
            }),
            None => {
                let (field, value_type) = default_field(spec);
                let hint = match (&field, spec["default"]["values"].as_array()) {
                    (Field::Unset, Some(values)) if spec["role"] != "file" => format!(
                        "default {}",
                        values
                            .iter()
                            .map(|value| {
                                value
                                    .as_f64()
                                    .map_or_else(|| value.to_string(), |n| n.to_string())
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                    _ => hint(spec),
                };
                rows.push(Row {
                    name: spec["name"].as_str().unwrap_or_default().to_owned(),
                    value_type,
                    implementation: implementation.clone(),
                    declared: false,
                    known: true,
                    required,
                    texture: lineage(spec),
                    texture_kind: spec["texture"]["kind"].as_str().map(str::to_owned),
                    value_types: spec_value_types(spec),
                    file: spec["role"] == "file",
                    group: group(spec),
                    when: when(spec),
                    choices: strings(&spec["choices"]),
                    suggestions: strings(&spec["suggestions"]),
                    field,
                    ignored: None,
                    hint,
                    parameter: Value::Null,
                });
            }
        }
    }
    for parameter in &declared {
        let name = parameter["name"].as_str().unwrap_or_default();
        if schema.iter().any(|spec| matches(spec, name)) {
            continue;
        }
        rows.push(Row {
            name: name.to_owned(),
            value_type: parameter["value_type"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            implementation: implementation.clone(),
            declared: true,
            known: false,
            required: false,
            texture: None,
            texture_kind: None,
            value_types: Vec::new(),
            file: false,
            group: None,
            when: Vec::new(),
            choices: Vec::new(),
            suggestions: Vec::new(),
            field: field(parameter, false),
            ignored: None,
            hint: String::new(),
            parameter: parameter.clone(),
        });
    }
    mark_conductor_ignored(&mut rows, &implementation);
    rows
}

/// A conductor page whose reflectance is declared greys its eta/k
/// rows — r4 reads the reflectance, not the curves (`unused:
/// reflectance`), or refuses the both-declared pair
/// (materials/conductor.rs). Both eta rows and k rows of the page's
/// prefix carry the hint, declared or not: r4 reads neither of them.
fn mark_conductor_ignored(rows: &mut [Row], implementation: &str) {
    let prefix = match implementation {
        "conductor" => "",
        "coatedconductor" => "conductor.",
        _ => return,
    };
    if !rows
        .iter()
        .any(|row| row.declared && row.name == "reflectance")
    {
        return;
    }
    let metal_row = |row: &Row, name: &str| row.name == format!("{prefix}{name}");
    let hint = if rows
        .iter()
        .any(|row| row.declared && (metal_row(row, "eta") || metal_row(row, "k")))
    {
        REFLECTANCE_WITH_ETA_K
    } else {
        UNUSED_REFLECTANCE
    };
    for row in rows {
        if metal_row(row, "eta") || metal_row(row, "k") {
            row.ignored = Some(hint);
        }
    }
}

/// The wire values of a field edit, or `None` when the text does not parse.
pub fn values(field: &Field, texts: &[String]) -> Option<Value> {
    let number = |text: &String| text.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    Some(match field {
        Field::Number { integer: true, .. } => {
            // The core takes only canonical decimal strings.
            let n = texts.first()?.trim().parse::<i64>().ok()?;
            json!({"kind":"integers","value":[n.to_string()]})
        }
        Field::Number { integer: false, .. } => {
            json!({"kind":"floats","value":[number(texts.first()?)?]})
        }
        Field::Rgb(_) | Field::Vector(_) => {
            let xyz = texts.iter().map(number).collect::<Option<Vec<_>>>()?;
            (xyz.len() == 3).then_some(())?;
            json!({"kind":"floats","value":xyz})
        }
        Field::Spectrum(_) => json!({"kind":"builtin_spectra","value":[texts.first()?]}),
        Field::Bool(_) => json!({"kind":"booleans","value":[texts.first()? == "true"]}),
        Field::Text(_) => json!({"kind":"strings","value":[texts.first()?]}),
        _ => return None,
    })
}

/// `set_parameter` declaring or replacing `row` with `values`. An explicit
/// color space on the declared value is kept.
pub fn set_edit(target: &Value, row: &Row, values: Value) -> Value {
    let color_space = row.parameter["color_space"]["value"].clone();
    json!({"kind":"set_parameter","target":target,"name":row.name,"value_type":row.value_type,
           "value":{"name":row.name,"value_type":row.value_type,
                    "color_space":color_space,"values":values}})
}

/// Build the single typed edit used by either mix-material chooser. An
/// undeclared mix starts with the chosen material in both operands.
pub fn mix_material_edit(target: &Value, ids: &[String], index: usize, chosen: &str) -> Value {
    let mut materials = match ids {
        [first, second, ..] => vec![first.clone(), second.clone()],
        [only] => vec![only.clone(), only.clone()],
        [] => vec![chosen.to_owned(), chosen.to_owned()],
    };
    if index < materials.len() {
        materials[index] = chosen.to_owned();
    }
    json!({"kind":"set_parameter","target":target,"name":"materials","value_type":"string",
           "value":{"name":"materials","value_type":"string","color_space":null,
                    "values":{"kind":"resources","value":{"kind":"material","ids":materials}}}})
}

/// `set_parameter` removing the declaration, so r4's default applies again.
/// Never offered for undeclared or required rows.
pub fn reset_edit(target: &Value, row: &Row) -> Option<Value> {
    (row.declared && !row.required).then(|| {
        json!({"kind":"set_parameter","target":target,"name":row.name,
               "value_type":row.value_type,"value":null})
    })
}

/// The row a slot's edit builds on: the declared spelling (an alias
/// stays an alias) else the slot's own name, and the flags the
/// colour-edits read — the declaration flag and the table's required
/// flag, whose default-lessness is exactly why the colour edits refuse
/// to remove one. Everything else stays inert — the builders here read
/// the name, the flags and the value types only.
fn slot_row(parameter: &str, declared: Option<(&str, &str)>, required: bool) -> Row {
    Row {
        name: declared.map_or_else(|| parameter.to_owned(), |(name, _)| name.to_owned()),
        value_type: declared.map_or_else(
            || "spectrum".to_owned(),
            |(_, value_type)| value_type.to_owned(),
        ),
        declared: declared.is_some(),
        implementation: String::new(),
        known: false,
        required,
        texture: None,
        texture_kind: None,
        value_types: Vec::new(),
        file: false,
        group: None,
        when: Vec::new(),
        choices: Vec::new(),
        suggestions: Vec::new(),
        field: Field::Unset,
        ignored: None,
        hint: String::new(),
        parameter: Value::Null,
    }
}

/// The `project.apply` edits that point a spectrum-capable slot at
/// one of the project's SpectrumData resources — the very payloads the
/// colour dialog's Spectrum tab builds (`colour_edits`): a declaration
/// of another type is replaced first, both edits riding one transaction
/// (one Undo step). `declared` is the slot's declaration as the graph
/// row spells it (name, value type), None when the slot holds nothing
/// yet; the edits name the declaration's own spelling, else the slot.
pub fn spectrum_connect_edits(
    target: &Value,
    parameter: &str,
    declared: Option<(&str, &str)>,
    required: bool,
    resource_id: &str,
) -> Vec<Value> {
    let row = slot_row(parameter, declared, required);
    colour_edits(
        target,
        &row,
        "spectrum",
        json!({"kind":"resources","value":{"kind":"spectrum_data","ids":[resource_id]}}),
    )
}

/// The value node's "Change to Spectrum texture": the one edit
/// `connect_texture` with `create` — core drops the declared value and
/// wires the new texture in the same transaction, so one Undo step
/// returns the slot's old value.
pub fn value_texture_edits(
    target: &Value,
    parameter: &str,
    declared: Option<(&str, &str)>,
    required: bool,
    implementation: &str,
) -> Vec<Value> {
    let row = slot_row(parameter, declared, required);
    vec![create_edit(target, &row, implementation)]
}

/// The value node's "Disconnect": the slot returns to r4's default by
/// removing its declaration, named as it was declared — the same
/// removal the colour edits spell. A required operand has no default
/// to return to, so it offers nothing.
pub fn value_disconnect_edits(
    target: &Value,
    declared: (&str, &str),
    required: bool,
) -> Vec<Value> {
    let row = slot_row(declared.0, Some(declared), required);
    reset_edit(target, &row).into_iter().collect()
}

/// The value node's replacement menu:
/// `change_texture` offers one item per implementation a new texture
/// may take, `spectra` one per project SpectrumData (the declared value
/// replaced first, both edits in one apply), `disconnect` the one
/// removal — absent for a required operand, which has no r4 default to
/// return to. Every item applies its own edits as one transaction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValueNodeMenu {
    pub change_texture: Vec<(String, Vec<Value>)>,
    pub spectra: Vec<(String, Vec<Value>)>,
    pub disconnect: Option<Vec<Value>>,
}

/// The menu of the value node on `declared` (the spellings the slot was
/// declared with) — `required` is the table's own flag of the slot the
/// declaration sits on, from the graph's input row — against the
/// project's Contents `items` and the implementations a new texture of
/// the slot's lineage may take.
pub fn value_node_menu(
    target: &Value,
    declared: (&str, &str),
    required: bool,
    items: &Value,
    implementations: &[String],
) -> ValueNodeMenu {
    let disconnect = value_disconnect_edits(target, declared, required);
    ValueNodeMenu {
        change_texture: implementations
            .iter()
            .map(|implementation| {
                (
                    format!("New {implementation}"),
                    value_texture_edits(
                        target,
                        declared.0,
                        Some(declared),
                        required,
                        implementation,
                    ),
                )
            })
            .collect(),
        spectra: spectrum_data_choices(items)
            .into_iter()
            .map(|(name, id)| {
                (
                    name,
                    spectrum_connect_edits(target, declared.0, Some(declared), required, &id),
                )
            })
            .collect(),
        disconnect: (!disconnect.is_empty()).then_some(disconnect),
    }
}

/// The slot row of a graph input, as the placement builders read it:
/// the tab rule rides the row's `value_types` (what the graph's slot
/// rows carry since the value-node wires), the placement is a fresh
/// declaration — the input's own `required` rides along for the builders
/// that consult it, a placement never removes anything.
fn placed_row(input: &Value) -> Row {
    let mut row = slot_row(
        input["parameter"].as_str().unwrap_or_default(),
        None,
        input["required"] == true,
    );
    row.known = true;
    row.value_types = strings(&input["value_types"]);
    row
}

/// The shapes an empty slot may grow a value node in: the colour
/// dialog's own tab rule, read off the graph input row.
pub fn placement_tabs(input: &Value) -> SpectrumTabs {
    spectrum_tabs(&placed_row(input))
}

/// The edits that place `tab`'s value in an empty slot: the colour
/// dialog's tab defaults, declared under the slot's own name.
pub fn placement_edits(
    target: &Value,
    input: &Value,
    tab: SpectrumTab,
    items: &Value,
    builtins: &[String],
) -> Vec<Value> {
    let row = placed_row(input);
    tab_default_edits(target, &row, tab, items, builtins)
}

/// The slot menu's "Place …" items and its "Place Spectrum Data"
/// section: the flat items, one per value shape the row
/// allows and the project can give, and the submenu's entries, one per
/// project SpectrumData resource — each entry's edits are the very
/// ones the Inspector's Data tab applies. An item without a default to
/// give (no built-in spectrum in the table) is left out; a section with
/// nothing to list hides. All sections empty hides the + entirely.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacementMenu {
    /// One "Place <shape>" item per allowed value shape.
    pub items: Vec<(String, Vec<Value>)>,
    /// The project's Spectrum Data, sorted by name.
    pub spectra: Vec<(String, Vec<Value>)>,
}

impl PlacementMenu {
    /// Whether the slot keeps its + menu at all.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.spectra.is_empty()
    }
}

/// The + menu model of an empty spectrum-capable slot:
/// the "Place …" items for each shape the row's value types allow —
/// the colour dialog's own tab rule, read off the graph input row —
/// and the "Place Spectrum Data" entries when the core says the slot
/// takes SpectrumData nodes (`input["spectrum_data"]`).
pub fn placement_menu(
    target: &Value,
    input: &Value,
    items: &Value,
    builtins: &[String],
) -> PlacementMenu {
    let tabs = placement_tabs(input);
    let placement_items = [
        (SpectrumTab::Rgb, tabs.rgb, "Place RGB"),
        (SpectrumTab::Blackbody, tabs.blackbody, "Place Blackbody"),
        (SpectrumTab::Named, tabs.named, "Place Named spectrum"),
    ]
    .into_iter()
    .filter(|(_, allowed, _)| *allowed)
    .map(|(tab, _, label)| {
        (
            label.to_owned(),
            placement_edits(target, input, tab, items, builtins),
        )
    })
    .filter(|(_, edits)| !edits.is_empty())
    .collect();
    let spectra = if input["spectrum_data"] == true {
        spectrum_data_choices(items)
    } else {
        Vec::new()
    }
    .into_iter()
    .map(|(name, id)| {
        let parameter = input["parameter"].as_str().unwrap_or_default();
        let required = input["required"] == true;
        let edits = spectrum_connect_edits(target, parameter, None, required, &id);
        (name, edits)
    })
    .collect();
    PlacementMenu {
        items: placement_items,
        spectra,
    }
}

/// The project's Spectrum Data resources (name, id), sorted by name
/// — the Connect menu's spectrum section, beside the texture lineage's
/// own list.
pub fn spectrum_data_choices(items: &Value) -> Vec<(String, String)> {
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

/// The Connect menu's sections for `row`, the edits each
/// entry applies: the textures of the row's lineage (never the target
/// itself), the new-texture items — each one `connect_texture` — and the
/// project's Spectrum Data, each applying the colour dialog's own
/// declaration edits (`colour_edits` with the `spectrum` value type),
/// offered on every spectrum-capable row. All sections empty hides the
/// menu; two adjacent non-empty sections draw a separator between them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnectMenu {
    pub textures: Vec<(String, Vec<Value>)>,
    pub create: Vec<(String, Vec<Value>)>,
    pub spectra: Vec<(String, Vec<Value>)>,
}

impl ConnectMenu {
    /// The sections top to bottom, empty ones left out.
    pub fn sections(&self) -> impl Iterator<Item = &[(String, Vec<Value>)]> {
        [&self.textures, &self.create, &self.spectra]
            .into_iter()
            .filter(|section| !section.is_empty())
            .map(|section| section.as_slice())
    }
}

/// Build the Connect menu of `row` against the project's
/// Contents `items`; `create` names the implementations a new texture
/// of the row's lineage may take (empty for a row without one).
pub fn connect_menu(target: &Value, row: &Row, items: &Value, create: &[String]) -> ConnectMenu {
    let target_id = target["id"].as_str().unwrap_or_default();
    let textures = connect_choices(row, items, target_id)
        .into_iter()
        .map(|(name, id)| (name, vec![connect_edit(target, row, &id)]))
        .collect();
    let create = create
        .iter()
        .map(|implementation| {
            (
                format!("New {implementation}"),
                vec![create_edit(target, row, implementation)],
            )
        })
        .collect();
    let spectra = spectrum_sources(row)
        .map(|_| spectrum_data_choices(items))
        .unwrap_or_default()
        .into_iter()
        .map(|(name, id)| {
            (
                name,
                colour_edits(
                    target,
                    row,
                    "spectrum",
                    json!({"kind":"resources","value":{"kind":"spectrum_data","ids":[id]}}),
                ),
            )
        })
        .collect();
    ConnectMenu {
        textures,
        create,
        spectra,
    }
}

/// The textures `row` may connect to (name, id), sorted by name: the
/// row's lineage, never the target itself (that loop is refused anyway).
pub fn connect_choices(row: &Row, items: &Value, target_id: &str) -> Vec<(String, String)> {
    let Some(lineage) = &row.texture else {
        return Vec::new();
    };
    let mut choices = items
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, item)| {
            id.as_str() != target_id
                && item["resource_kind"] == "texture"
                && item["texture_value_type"] == lineage.as_str()
        })
        .map(|(id, item)| (item["name"].as_str().unwrap_or(id).to_owned(), id.clone()))
        .collect::<Vec<_>>();
    choices.sort();
    choices
}

/// `connect_texture` to an existing texture.
pub fn connect_edit(target: &Value, row: &Row, texture: &str) -> Value {
    json!({"kind":"connect_texture","target":target,"parameter":row.name,"texture":texture})
}

/// `connect_texture` creating a new `implementation` texture for the slot.
pub fn create_edit(target: &Value, row: &Row, implementation: &str) -> Value {
    json!({"kind":"connect_texture","target":target,"parameter":row.name,
           "create":{"implementation":implementation,"label":format!("{} {implementation}", row.name)}})
}

/// The row's removal button: `disconnect_texture` for a connection the
/// table knows (core checks the slot), else `reset_edit`'s plain removal —
/// a texture reference the table does not know stays removable.
pub fn removal_edit(target: &Value, row: &Row) -> Option<Value> {
    if row.required {
        return None;
    }
    if matches!(row.field, Field::Texture(_)) && connectable(target, row) {
        Some(disconnect_edit(target, row))
    } else {
        reset_edit(target, row)
    }
}

/// Whether `connect_texture` / `disconnect_texture` apply to `row`: a
/// texture input the table knows, on a material or texture resource or a
/// scene graph entity (a shape's alpha mask).
pub fn connectable(target: &Value, row: &Row) -> bool {
    row.texture.is_some() && matches!(target["kind"].as_str(), Some("resource" | "node"))
}

/// The image assets a texture's file row may take (name, id), sorted by
/// name. Only texture resources choose here; other file rows keep links.
pub fn image_choices(target: &Value, row: &Row, items: &Value) -> Vec<(String, String)> {
    let texture = target["kind"] == "resource"
        && target["id"]
            .as_str()
            .is_some_and(|id| items[id]["resource_kind"] == "texture");
    if !row.file || !texture {
        return Vec::new();
    }
    image_assets(items)
}

/// The image assets a texture input may connect directly (name, id):
/// core reuses or creates an imagemap of the row's lineage.
pub fn connect_image_choices(target: &Value, row: &Row, items: &Value) -> Vec<(String, String)> {
    if !connectable(target, row) {
        return Vec::new();
    }
    image_assets(items)
}

/// `connect_texture` with an image asset.
pub fn connect_image_edit(target: &Value, row: &Row, asset: &str) -> Value {
    json!({"kind":"connect_texture","target":target,"parameter":row.name,"image":asset})
}

/// The project's image assets (name, id), sorted by name.
fn image_assets(items: &Value) -> Vec<(String, String)> {
    let mut choices = items
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, item)| {
            item["kind"] == "asset"
                && item["media_type"]
                    .as_str()
                    .is_some_and(|media| media.starts_with("image/"))
        })
        .map(|(id, item)| (item["name"].as_str().unwrap_or(id).to_owned(), id.clone()))
        .collect::<Vec<_>>();
    choices.sort();
    choices
}

/// `set_parameter` pointing a file row at `asset`.
pub fn image_edit(target: &Value, row: &Row, asset: &str) -> Value {
    let row = Row {
        value_type: "string".into(),
        ..row.clone()
    };
    set_edit(target, &row, json!({"kind":"assets","value":[asset]}))
}

/// `disconnect_texture`: r4's default applies again.
pub fn disconnect_edit(target: &Value, row: &Row) -> Value {
    json!({"kind":"disconnect_texture","target":target,"parameter":row.name})
}

/// The node editor's row context menu uses the same typed removal as the
/// Inspector, but only offers it for a connected, optional texture input.
pub fn row_disconnect_menu_edit(
    target: &Value,
    parameter: &str,
    required: bool,
    connected: bool,
    read_only: bool,
    texture_input: bool,
    declared_type: Option<&str>,
) -> Option<Value> {
    if read_only || required || !connected {
        return None;
    }
    if texture_input {
        Some(json!({"kind":"disconnect_texture","target":target,"parameter":parameter}))
    } else {
        declared_type.map(|value_type| {
            json!({"kind":"set_parameter","target":target,
            "name":parameter,"value_type":value_type,"value":null})
        })
    }
}

/// A resource-card menu action for one outgoing graph edge.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDisconnectMenuItem {
    pub label: String,
    pub edit: Value,
    pub displaced: String,
}

/// Disconnect actions for edges from `source` to editable texture inputs
/// in this graph. Required inputs (including mix material positions) are
/// intentionally omitted, matching the Inspector's removal rules.
pub fn node_disconnect_menu(
    graph: &Value,
    source: &str,
    read_only: bool,
) -> Vec<NodeDisconnectMenuItem> {
    if read_only {
        return Vec::new();
    }
    graph["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|edge| edge["from"] == source)
        .filter_map(|edge| {
            let target_id = edge["to"].as_str()?;
            let parameter = edge["parameter"].as_str()?;
            let target = graph["nodes"]
                .as_array()?
                .iter()
                .find(|node| node["id"] == target_id)?;
            let input = target["inputs"]
                .as_array()?
                .iter()
                .find(|input| input["parameter"] == parameter)?;
            let required = input["required"] == true || input["accepts"] == "material";
            let texture_input = input["connectable"] == true;
            let declared_type = input["declared_type"].as_str();
            if required || (!texture_input && declared_type.is_none()) {
                return None;
            }
            let label = target["label"].as_str().unwrap_or(target_id);
            let slot = edge["index"].as_u64().map_or_else(
                || parameter.to_owned(),
                |index| format!("{parameter} {}", index + 1),
            );
            Some(NodeDisconnectMenuItem {
                label: format!("Disconnect from {label} · {slot}"),
                edit: row_disconnect_menu_edit(
                    &json!({"kind":"resource","id":target_id}),
                    parameter,
                    required,
                    true,
                    false,
                    texture_input,
                    declared_type,
                )?,
                displaced: source.to_owned(),
            })
        })
        .collect()
}

pub fn node_delete_allowed(source: &str, root: &str, read_only: bool) -> bool {
    !read_only && source != root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, value_types: Value, default: Value, default_kind: &str) -> Value {
        json!({"name":name,"aliases":[],"value_types":value_types,"default":default,
               "default_kind":default_kind,"default_depends":null,"texture":null,
               "role":"value","required":false})
    }

    fn declared(name: &str, value_type: &str, kind: &str, value: Value) -> Value {
        json!({"name":name,"value_type":value_type,"color_space":{"kind":"inherit"},
               "value_count":"1","values":{"kind":kind,"value":value},"preview_truncated":false})
    }

    #[test]
    fn table_rows_show_declared_values_and_greyed_defaults() {
        let mut zmax = spec("zmax", json!(["float"]), Value::Null, "depends");
        zmax["default_depends"] = json!({"parameter":"radius","operation":"same"});
        let page = json!({
            "parameters":[
                declared("radius","float","floats",json!([{"kind":"finite","value":2.0}])),
                declared("mystery","integer","integers",json!(["7"])),
            ],
            "schema":[
                spec("radius",json!(["float"]),json!({"value_type":"float","values":[1.0]}),"constant"),
                zmax,
                spec("phimax",json!(["float"]),json!({"value_type":"float","values":[360.0]}),"constant"),
            ],
        });
        let rows = rows(&page);
        let names = rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["radius", "zmax", "phimax", "mystery"]);
        assert!(rows[0].declared);
        assert_eq!(
            rows[0].field,
            Field::Number {
                integer: false,
                value: "2".into()
            }
        );
        // Undeclared: the derived default is a hint over an empty field.
        assert!(!rows[1].declared);
        assert_eq!(rows[1].hint, "follows radius");
        assert_eq!(
            rows[1].field,
            Field::Number {
                integer: false,
                value: String::new()
            }
        );
        // Undeclared with a constant default shows it.
        assert_eq!(
            rows[2].field,
            Field::Number {
                integer: false,
                value: "360".into()
            }
        );
        // Declared but not in the table comes last, typed when it can be.
        assert!(rows[3].declared && !rows[3].known);
        assert_eq!(
            rows[3].field,
            Field::Number {
                integer: true,
                value: "7".into()
            }
        );
    }

    /// The Film page shows the schema's unset rows with r4's defaults greyed,
    /// the output name stays a typeable plain string (never a managed
    /// asset), and choice rows carry their choices.
    #[test]
    fn film_rows_show_greyed_defaults_and_a_typeable_filename() {
        let mut sensor = spec("sensor", json!(["string"]), Value::Null, "constant");
        sensor["default"] = json!({"value_type":"string","values":["cie1931"]});
        sensor["choices"] = json!(["cie1931", "canon_eos_5d_mkiv"]);
        let filename = spec("filename", json!(["string"]), json!(null), "constant");
        let mut page = json!({
            "parameters":[
                declared("xresolution","integer","integers",json!([640])),
                declared("yresolution","integer","integers",json!([360])),
            ],
            "schema":[
                filename,
                spec("xresolution",json!(["integer"]),json!({"value_type":"integer","values":[1280]}),"constant"),
                spec("yresolution",json!(["integer"]),json!({"value_type":"integer","values":[720]}),"constant"),
                spec("cropwindow",json!(["float"]),Value::Null,"none"),
                sensor,
            ],
            "implementation":"rgb",
        });
        // A constant default object rides the spec's default slot.
        page["schema"][0]["default"] = json!({"value_type":"string","values":["pbrt.exr"]});
        let rows = rows(&page);
        let names = rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "filename",
                "xresolution",
                "yresolution",
                "cropwindow",
                "sensor"
            ]
        );
        // The unset output name shows r4's default as typeable text.
        assert!(!rows[0].declared);
        assert!(!rows[0].file);
        assert_eq!(rows[0].field, Field::Text("pbrt.exr".into()));
        assert_eq!(rows[0].hint, "");
        // The declared values read back.
        assert_eq!(
            rows[1].field,
            Field::Number {
                integer: true,
                value: "640".into()
            }
        );
        // A row with no constant default reads as not set.
        assert!(!rows[3].declared);
        assert_eq!(rows[3].hint, "not set");
        // The sensor row keeps its choices.
        assert_eq!(rows[4].choices, ["cie1931", "canon_eos_5d_mkiv"]);
        assert_eq!(rows[4].field, Field::Text("cie1931".into()));
    }

    #[test]
    fn fields_follow_the_declared_type() {
        let field_of = |p: Value| field(&p, false);
        assert_eq!(
            field_of(declared("Kd", "rgb", "floats", json!([0.1, 0.2, 0.3]))),
            Field::Rgb([0.1, 0.2, 0.3])
        );
        assert_eq!(
            field_of(declared(
                "eta",
                "spectrum",
                "builtin_spectra",
                json!(["metal-Cu-eta"])
            )),
            Field::Spectrum("metal-Cu-eta".into())
        );
        assert_eq!(
            field_of(declared("twosided", "bool", "booleans", json!([true]))),
            Field::Bool(true)
        );
        assert_eq!(
            field_of(declared("mapping", "string", "strings", json!(["uv"]))),
            Field::Text("uv".into())
        );
        let texture = json!({"name":"reflectance","value_type":"texture",
            "values":{"kind":"resources","resource_kind":"texture","ids":["t"]}});
        assert_eq!(field_of(texture), Field::Texture("t".into()));
        let materials = json!({"name":"materials","value_type":"string",
            "values":{"kind":"resources","resource_kind":"material","ids":["a","b"]}});
        assert_eq!(
            field_of(materials),
            Field::Materials(vec!["a".into(), "b".into()])
        );
        // Point lists and truncated previews keep the JSON form.
        assert_eq!(
            field_of(declared("P", "point3", "floats", json!([0, 0, 0, 1, 0, 0]))),
            Field::Json
        );
        let mut long = declared("radius", "float", "floats", json!([1.0]));
        long["preview_truncated"] = json!(true);
        assert_eq!(field_of(long), Field::Json);
    }

    /// An imagemap starts without an image (New menu) and chooses one of
    /// the project's image assets; other files and non-textures keep links.
    #[test]
    fn texture_file_rows_choose_among_image_assets() {
        let mut filename = spec("filename", json!(["string"]), Value::Null, "none");
        filename["role"] = json!("file");
        filename["required"] = json!(true);
        let page = json!({"parameters":[],"schema":[filename]});
        let row = rows(&page).remove(0);
        assert!(row.file);
        let items = json!({
            "tex": {"kind":"resource","resource_kind":"texture","name":"Pending"},
            "b": {"kind":"asset","media_type":"image/png","name":"wood.png"},
            "a": {"kind":"asset","media_type":"image/x-exr","name":"albedo.exr"},
            "m": {"kind":"asset","media_type":"application/x-ply","name":"toy.ply"},
            "mat": {"kind":"resource","resource_kind":"material","name":"Mat"}
        });
        let target = json!({"kind":"resource","id":"tex"});
        assert_eq!(
            image_choices(&target, &row, &items),
            [
                ("albedo.exr".to_owned(), "a".to_owned()),
                ("wood.png".to_owned(), "b".to_owned())
            ]
        );
        assert!(image_choices(&json!({"kind":"resource","id":"mat"}), &row, &items).is_empty());
        assert!(image_choices(&json!({"kind":"node","id":"tex"}), &row, &items).is_empty());
        assert_eq!(
            image_edit(&target, &row, "b"),
            json!({"kind":"set_parameter","target":target,"name":"filename","value_type":"string",
                   "value":{"name":"filename","value_type":"string","color_space":null,
                            "values":{"kind":"assets","value":["b"]}}})
        );
    }

    #[test]
    fn undeclared_files_and_point_lists_cannot_be_typed_in() {
        let mut normalmap = spec(
            "normalmap",
            json!(["string"]),
            json!({"value_type":"string","values":[""]}),
            "constant",
        );
        normalmap["role"] = json!("file");
        let page = json!({"parameters":[],"schema":[
            normalmap, spec("P",json!(["point3"]),Value::Null,"none")]});
        let rows = rows(&page);
        assert_eq!(rows[0].field, Field::Unset);
        assert_eq!(rows[0].hint, "file: not set");
        assert_eq!(rows[1].field, Field::Unset);
        assert_eq!(values(&Field::Unset, &["x".into()]), None);
    }

    #[test]
    fn texture_inputs_offer_textures_of_their_lineage() {
        let mut reflectance = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        reflectance["texture"] = json!({"kind":"spectrum_albedo"});
        let mut roughness = spec(
            "roughness",
            json!(["float"]),
            json!({"value_type":"float","values":[0.0]}),
            "constant",
        );
        roughness["texture"] = json!({"kind":"float"});
        let page = json!({"parameters":[],"schema":[reflectance, roughness,
            spec("eta",json!(["float"]),Value::Null,"none")]});
        let rows = rows(&page);
        assert_eq!(rows[0].texture.as_deref(), Some("spectrum"));
        assert_eq!(rows[1].texture.as_deref(), Some("float"));
        assert_eq!(rows[2].texture, None);
        let items = json!({
            "s1":{"id":"s1","name":"checks","kind":"resource","resource_kind":"texture","texture_value_type":"spectrum"},
            "f1":{"id":"f1","name":"grime","kind":"resource","resource_kind":"texture","texture_value_type":"float"},
            "me":{"id":"me","name":"self","kind":"resource","resource_kind":"texture","texture_value_type":"spectrum"},
            "m1":{"id":"m1","name":"wall","kind":"resource","resource_kind":"material"},
        });
        // Same lineage only, never the target itself, sorted by name.
        assert_eq!(
            connect_choices(&rows[0], &items, "me"),
            [("checks".to_owned(), "s1".to_owned())]
        );
        assert_eq!(
            connect_choices(&rows[1], &items, "me"),
            [("grime".to_owned(), "f1".to_owned())]
        );
        assert!(connect_choices(&rows[2], &items, "me").is_empty());
        let target = json!({"kind":"resource","id":"me"});
        assert_eq!(
            connect_edit(&target, &rows[0], "s1"),
            json!({"kind":"connect_texture","target":target,"parameter":"reflectance","texture":"s1"})
        );
        assert_eq!(
            create_edit(&target, &rows[0], "marble"),
            json!({"kind":"connect_texture","target":target,"parameter":"reflectance",
                   "create":{"implementation":"marble","label":"reflectance marble"}})
        );
        assert_eq!(
            disconnect_edit(&target, &rows[0]),
            json!({"kind":"disconnect_texture","target":target,"parameter":"reflectance"})
        );
    }

    /// The edits a SpectrumData node drop (and the Connect menu's
    /// spectrum section) applies — `colour_edits`' own payloads: an unset
    /// slot declares the curve, a same-type declaration replaces in
    /// place, and a declaration of another type (rgb, a texture
    /// reference) is removed first by its own spelling, one transaction,
    /// one Undo step. Every payload deserialises as the real wire edit
    /// type.
    #[test]
    fn spectrum_connect_edits_name_the_declaration_they_replace() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let curve = "00000000-0000-0000-0000-0000000000bb";
        let sets = |name: &str| {
            json!({"kind":"set_parameter","target":target,"name":name,"value_type":"spectrum",
                   "value":{"name":name,"value_type":"spectrum","color_space":null,
                            "values":{"kind":"resources",
                                      "value":{"kind":"spectrum_data","ids":[curve]}}}})
        };
        // An unset slot declares the curve.
        assert_eq!(
            spectrum_connect_edits(&target, "k", None, false, curve),
            vec![sets("k")]
        );
        // A declared spectrum value replaces in place: same type, one
        // edit.
        assert_eq!(
            spectrum_connect_edits(&target, "k", Some(("k", "spectrum")), false, curve),
            vec![sets("k")]
        );
        // Cross-type: the rgb declaration is removed first, both edits
        // riding one transaction.
        assert_eq!(
            spectrum_connect_edits(
                &target,
                "reflectance",
                Some(("reflectance", "rgb")),
                false,
                curve
            ),
            vec![
                json!({"kind":"set_parameter","target":target,"name":"reflectance",
                       "value_type":"rgb","value":null}),
                sets("reflectance"),
            ]
        );
        // An alias-declared slot is replaced under its own spelling —
        // removing `tex` would touch nothing and leave both operands.
        assert_eq!(
            spectrum_connect_edits(&target, "tex", Some(("tex1", "texture")), true, curve),
            vec![
                json!({"kind":"set_parameter","target":target,"name":"tex1",
                       "value_type":"texture","value":null}),
                sets("tex1"),
            ],
            "a required operand is still replaceable — it only never removes to nothing"
        );
        for edits in [
            spectrum_connect_edits(&target, "k", None, false, curve),
            spectrum_connect_edits(&target, "k", Some(("k", "spectrum")), false, curve),
            spectrum_connect_edits(
                &target,
                "reflectance",
                Some(("reflectance", "rgb")),
                false,
                curve,
            ),
            spectrum_connect_edits(&target, "tex", Some(("tex1", "texture")), true, curve),
        ]
        .into_iter()
        .flatten()
        {
            serde_json::from_value::<protocol::project::ProjectEdit>(edits.clone())
                .unwrap_or_else(|error| panic!("{error}: {edits}"));
        }
    }

    /// The value node's menu, the R the adds: every item
    /// names its own spelling, and every payload deserialises as the
    /// wire edit type. "Change to Spectrum texture"
    /// creates the texture and connects it in the one edit
    /// `connect_texture` with `create` — one transaction, one Undo step.
    #[test]
    fn value_node_menu_edits_change_to_a_created_spectrum_texture() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let edits = value_texture_edits(&target, "k", Some(("k", "rgb")), false, "imagemap");
        assert_eq!(
            edits,
            vec![
                json!({"kind":"connect_texture","target":target,"parameter":"k",
                   "create":{"implementation":"imagemap","label":"k imagemap"}})
            ],
            "one edit, one transaction"
        );
        // An alias-declared value changes under its own spelling.
        let aliased = value_texture_edits(&target, "tex", Some(("tex1", "rgb")), true, "bumpmap");
        assert_eq!(
            aliased,
            vec![
                json!({"kind":"connect_texture","target":target,"parameter":"tex1",
                   "create":{"implementation":"bumpmap","label":"tex1 bumpmap"}})
            ]
        );
        for edit in edits.into_iter().chain(aliased) {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
    }

    /// "Change to Spectrum texture" on an unset slot declares the
    /// connection under the row's own name — no removal first, the slot
    /// held nothing to remove.
    #[test]
    fn value_texture_edits_on_an_unset_slot_name_the_row() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let edits = value_texture_edits(&target, "k", None, false, "imagemap");
        assert_eq!(
            edits,
            vec![
                json!({"kind":"connect_texture","target":target,"parameter":"k",
                   "create":{"implementation":"imagemap","label":"k imagemap"}})
            ]
        );
        serde_json::from_value::<protocol::project::ProjectEdit>(edits[0].clone())
            .expect("the connect deserialises as the wire edit type");
    }

    /// "Disconnect" returns the slot to r4's default: the one edit
    /// removes the declaration, naming the spelling it was declared
    /// with — the same shape the colour-edits removals use. A required
    /// operand has no default to return to (the scale texture's
    /// ±tex/tex1): its declaration stays, the removal is not offered.
    #[test]
    fn value_node_disconnect_edits_drop_the_declaration_by_its_spelling() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let edits = value_disconnect_edits(&target, ("k", "rgb"), false);
        assert_eq!(
            edits,
            vec![json!({"kind":"set_parameter","target":target,"name":"k",
                   "value_type":"rgb","value":null})]
        );
        // An alias keeps its own spelling here too.
        let aliased = value_disconnect_edits(&target, ("tex1", "blackbody"), false);
        assert_eq!(
            aliased,
            vec![json!({"kind":"set_parameter","target":target,"name":"tex1",
                   "value_type":"blackbody","value":null})]
        );
        for edit in edits.into_iter().chain(aliased) {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
        // The required operand keeps its declaration: no removal at all.
        assert!(
            value_disconnect_edits(&target, ("tex1", "blackbody"), true).is_empty(),
            "a required operand has no default to return to"
        );
    }

    /// The value node's full menu: the create section, the project's
    /// Spectrum Data, and the one disconnect item. Every payload rides
    /// its own apply (one Undo step) and deserialises as the wire type.
    /// A declared value is replaced by a SpectrumData in two edits of
    /// one transaction — the removal first, naming its spelling.
    #[test]
    fn value_node_menu_lists_creates_spectra_and_disconnect() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let items = json!({
            "00000000-0000-0000-0000-00000000ab00":{"kind":"resource","resource_kind":"spectrum_data","name":"Au.k.spd"},
            "t1":{"kind":"resource","resource_kind":"texture","name":"checker"},
        });
        let menu = value_node_menu(
            &target,
            ("k", "rgb"),
            false,
            &items,
            &["imagemap".to_owned(), "constant".to_owned()],
        );
        assert_eq!(
            menu.change_texture
                .iter()
                .map(|(label, _)| label.clone())
                .collect::<Vec<_>>(),
            ["New imagemap".to_owned(), "New constant".to_owned()]
        );
        assert_eq!(
            menu.spectra
                .iter()
                .map(|(label, _)| label.clone())
                .collect::<Vec<_>>(),
            ["Au.k.spd".to_owned()]
        );
        // Creating a texture is the one connect edit; attaching a
        // SpectrumData replaces the declared value first, both edits
        // in one apply.
        assert_eq!(menu.change_texture[0].1.len(), 1);
        assert_eq!(menu.spectra[0].1.len(), 2, "remove rgb, then declare");
        let disconnect = menu.disconnect.expect("a value node can disconnect");
        for (_, edits) in menu.change_texture.iter().chain(&menu.spectra) {
            for edit in edits.clone() {
                serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                    .unwrap_or_else(|error| panic!("{error}: {edit}"));
            }
        }
        for edit in disconnect {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
        // An alias-declared value names its own spelling everywhere.
        let aliased = value_node_menu(&target, ("tex1", "spectrum"), false, &items, &[]);
        assert_eq!(
            aliased.disconnect.unwrap(),
            vec![json!({"kind":"set_parameter","target":target,"name":"tex1",
                   "value_type":"spectrum","value":null})]
        );
        assert_eq!(
            aliased.spectra[0].1.len(),
            1,
            "same type: the declaration replaces in place"
        );
        // A required operand (the scale texture's tex, declared under
        // its tex1 alias) keeps every replacement — a new texture still
        // creates, a SpectrumData still replaces — but Disconnect, whose
        // removal would leave the texture without its operand.
        let required = value_node_menu(
            &target,
            ("tex1", "spectrum"),
            true,
            &items,
            &["imagemap".to_owned()],
        );
        assert!(required.disconnect.is_none(), "no removal to offer");
        assert_eq!(
            required.change_texture.len(),
            1,
            "a new texture is a replacement"
        );
        assert_eq!(required.spectra.len(), 1, "a SpectrumData is a replacement");
        for (_, edits) in required.change_texture.iter().chain(&required.spectra) {
            for edit in edits.clone() {
                serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                    .unwrap_or_else(|error| panic!("{error}: {edit}"));
            }
        }
    }

    /// The slot menu offers a value node per input shape the row allows
    /// (the colour dialog's own tab rule reads the graph's new
    /// `value_types`): conductor `k` takes RGB and a name but no
    /// blackbody; a float-only row offers nothing.
    #[test]
    fn empty_slots_place_value_nodes_of_each_allowed_shape() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let conductor_k = json!({"parameter":"k","accepts":"spectrum","connectable":true,
                                 "spectrum_data":true,"default":"metal-Cu-k",
                                 "value_types":["spectrum","rgb","float"]});
        let builtins = vec!["stdillum-D65".to_owned()];
        let items = json!({});
        let menu = placement_menu(&target, &conductor_k, &items, &builtins);
        let labels = menu
            .items
            .iter()
            .map(|(label, _)| label.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            ["Place RGB".to_owned(), "Place Named spectrum".to_owned()],
            "no blackbody on a row that does not take it"
        );
        let sets = |name: &str, value_type: &str, values: Value| {
            json!({"kind":"set_parameter","target":target,"name":name,"value_type":value_type,
                   "value":{"name":name,"value_type":value_type,"color_space":null,"values":values}})
        };
        assert_eq!(
            menu.items[0].1,
            vec![sets(
                "k",
                "rgb",
                json!({"kind":"floats","value":[1., 1., 1.]})
            )]
        );
        assert_eq!(
            menu.items[1].1,
            vec![sets(
                "k",
                "spectrum",
                json!({"kind":"builtin_spectra","value":["stdillum-D65"]})
            )]
        );
        // A row that allows blackbody offers it with the dialog's
        // default temperature, and a float-only row offers nothing.
        let blackbody_row = json!({"parameter":"L","accepts":"spectrum","connectable":true,
                                    "spectrum_data":true,"value_types":["spectrum","rgb","blackbody"]});
        let menu = placement_menu(&target, &blackbody_row, &items, &builtins);
        assert_eq!(
            menu.items
                .iter()
                .map(|(label, _)| label.clone())
                .collect::<Vec<_>>(),
            [
                "Place RGB".to_owned(),
                "Place Blackbody".to_owned(),
                "Place Named spectrum".to_owned(),
            ]
        );
        for edit in menu
            .items
            .into_iter()
            .chain(menu.spectra)
            .flat_map(|(_, edits)| edits)
        {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
        let float_row = json!({"parameter":"roughness","accepts":"float","connectable":true});
        assert!(placement_menu(&target, &float_row, &items, &builtins).is_empty());
    }

    /// The short muted label beside a spectrum row's
    /// colour box, naming what the value is — the resource's name for
    /// a Spectrum Data reference, the builtin name, "5500 K" for a
    /// blackbody; nothing for rgb, for unset rows, and for the shapes
    /// with no name of their own.
    #[test]
    fn spectrum_rows_carry_a_short_label_of_their_value() {
        let items = json!({
            "sd-1": {"kind":"resource","resource_kind":"spectrum_data","name":"Au.eta"},
        });
        let slot = || spec("k", json!(["spectrum", "rgb"]), Value::Null, "none");
        // Spectrum Data: the resource's name.
        {
            let rows = rows(&json!({"parameters":[
                json!({"name":"k","value_type":"spectrum","color_space":{"kind":"inherit"},
                       "values":{"kind":"resources","resource_kind":"spectrum_data","ids":["sd-1"]},
                       "preview_truncated":false}),
            ],"schema":[slot()]}));
            assert_eq!(value_label(&rows[0], &items), Some("Au.eta".into()));
        }
        // A named spectrum: the name.
        {
            let rows = rows(&json!({"parameters":[
                declared("k", "spectrum", "builtin_spectra", json!(["metal-Cu-eta"])),
            ],"schema":[slot()]}));
            assert_eq!(value_label(&rows[0], &items), Some("metal-Cu-eta".into()));
        }
        // A light's blackbody names its kelvin; its rgb carries nothing.
        {
            let light = |name: &str| {
                spec(
                    name,
                    json!(["spectrum", "rgb", "blackbody"]),
                    Value::Null,
                    "none",
                )
            };
            let rows = rows(&json!({"parameters":[
                declared("I", "blackbody", "floats", json!([5500])),
                declared("L", "rgb", "floats", json!([1., 1., 1.])),
            ],"schema":[light("I"), light("L")]}));
            // rows orders by schema: I first, then L.
            assert_eq!(value_label(&rows[0], &items), Some("5500 K".into()));
            assert_eq!(value_label(&rows[1], &items), None, "rgb keeps no label");
        }
        // An unset row keeps just its box.
        {
            let rows = rows(&json!({"parameters":[],"schema":[slot()]}));
            assert_eq!(value_label(&rows[0], &items), None);
        }
        // A texture connection has no label either: it keeps its link.
        {
            let rows = rows(&json!({"parameters":[
                json!({"name":"k","value_type":"texture","color_space":{"kind":"inherit"},
                       "values":{"kind":"resources","resource_kind":"texture","ids":["t1"]},
                       "preview_truncated":false}),
            ],"schema":[slot()]}));
            assert_eq!(value_label(&rows[0], &items), None);
        }
    }

    /// (item 8): a spectrum socket's + menu also offers the
    /// project's SpectrumData in a "Place Spectrum Data" section — each
    /// entry applying the same edit the colour dialog's Data tab makes,
    /// sorted by name, hidden when the project holds none, and absent
    /// on a socket that takes no SpectrumData nodes.
    #[test]
    fn the_slot_menu_places_spectrum_data_with_the_data_tab_edits() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let conductor_k = json!({"parameter":"k","accepts":"spectrum","connectable":true,
                                 "spectrum_data":true,"default":"metal-Cu-k",
                                 "value_types":["spectrum","rgb","float"]});
        let builtins = vec!["stdillum-D65".to_owned()];
        let items = json!({
            "00000000-0000-0000-0000-00000000ab01":
                {"kind":"resource","resource_kind":"spectrum_data","name":"b-side.spd"},
            "00000000-0000-0000-0000-00000000ab00":
                {"kind":"resource","resource_kind":"spectrum_data","name":"a-side.spd"},
            "t1": {"kind":"resource","resource_kind":"texture","name":"checker"},
        });
        let menu = placement_menu(&target, &conductor_k, &items, &builtins);
        assert_eq!(
            menu.spectra
                .iter()
                .map(|(label, _)| label.clone())
                .collect::<Vec<_>>(),
            ["a-side.spd".to_owned(), "b-side.spd".to_owned()],
            "sorted by name, textures stay out"
        );
        assert_eq!(
            menu.spectra[0].1,
            spectrum_connect_edits(
                &target,
                "k",
                None,
                false,
                "00000000-0000-0000-0000-00000000ab00",
            ),
            "the same edits the Inspector's Data tab makes"
        );
        // No SpectrumData in the project: the section hides, the flat
        // "Place …" items stay.
        let menu = placement_menu(&target, &conductor_k, &json!({}), &builtins);
        assert!(menu.spectra.is_empty());
        assert!(!menu.is_empty());
        // A socket that takes no SpectrumData nodes keeps the section
        // empty while its shapes still place their values.
        let no_data = json!({"parameter":"alpha","accepts":"spectrum","connectable":true,
                             "spectrum_data":false,"value_types":["spectrum","rgb"]});
        let menu = placement_menu(&target, &no_data, &items, &builtins);
        assert!(menu.spectra.is_empty());
        assert_eq!(menu.items.len(), 2);
    }

    /// The Connect menu lists the project's Spectrum Data resources,
    /// sorted by name (the texture section keeps its own lineage list).
    #[test]
    fn spectrum_section_lists_the_projects_spectrum_data() {
        let items = json!({
            "00000000-0000-0000-0000-00000000ab01":{"kind":"resource","resource_kind":"spectrum_data","name":"b-side.spd"},
            "00000000-0000-0000-0000-00000000ab00":{"kind":"resource","resource_kind":"spectrum_data","name":"a-side.spd"},
            "t1":{"kind":"resource","resource_kind":"texture","name":"checker"},
            "a1":{"kind":"asset","media_type":"image/png","name":"wood.png"},
        });
        assert_eq!(
            spectrum_data_choices(&items),
            [
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
    }

    #[test]
    fn removal_disconnects_only_table_texture_inputs() {
        let target = json!({"kind":"resource","id":"me"});
        let connected = json!({"name":"reflectance","value_type":"texture",
            "values":{"kind":"resources","resource_kind":"texture","ids":["t"]},"preview_truncated":false});
        let mut reflectance = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        reflectance["texture"] = json!({"kind":"spectrum_albedo"});
        let known = &rows(&json!({"parameters":[connected.clone()],"schema":[reflectance]}))[0];
        assert_eq!(
            removal_edit(&target, known).unwrap(),
            disconnect_edit(&target, known)
        );
        // A texture reference the table does not know: core would refuse
        // disconnect_texture, so it stays a plain removal.
        // A scene-graph entity's table texture input
        // connects and disconnects like a material's.
        let node = json!({"kind":"node","id":"n"});
        assert_eq!(
            removal_edit(&node, known).unwrap(),
            disconnect_edit(&node, known)
        );
        assert!(connectable(&node, known));
        assert!(connectable(&target, known));
        // A required operand offers no removal at all.
        let mut required = known.clone();
        required.required = true;
        assert_eq!(removal_edit(&target, &required), None);
        let unknown = &rows(&json!({"parameters":[connected],"schema":[]}))[0];
        assert_eq!(unknown.field, Field::Texture("t".into()));
        assert_eq!(
            removal_edit(&target, unknown).unwrap(),
            json!({"kind":"set_parameter","target":target,"name":"reflectance",
                   "value_type":"texture","value":null})
        );
    }

    #[test]
    fn integers_go_out_canonical() {
        let field = Field::Number {
            integer: true,
            value: String::new(),
        };
        // The core refuses non-canonical integer strings.
        assert_eq!(
            values(&field, &[" 007 ".into()]).unwrap(),
            json!({"kind":"integers","value":["7"]})
        );
        assert_eq!(
            values(&field, &["+4".into()]).unwrap(),
            json!({"kind":"integers","value":["4"]})
        );
    }

    #[test]
    fn multi_value_integer_parameters_remain_array_editor_rows() {
        let parameter = declared("indices", "integer", "integers", json!(["1", "2", "3"]));
        assert_eq!(field(&parameter, false), Field::Json);
    }

    #[test]
    fn an_undeclared_tuple_default_edits_as_a_vector() {
        // A spot light's from / to: a point3 default shows in x / y / z.
        let page = json!({"parameters":[],"schema":[spec("from",json!(["point3"]),
            json!({"value_type":"point3","values":[0.0,0.0,0.0]}),"constant")]});
        let row = &rows(&page)[0];
        assert_eq!(row.field, Field::Vector([0.0; 3]));
        assert_eq!(row.value_type, "point3");
    }

    #[test]
    fn an_integer_default_shows_its_value() {
        let page = json!({"parameters":[],"schema":[spec("dimension",json!(["integer"]),
            json!({"value_type":"integer","values":[2]}),"constant")]});
        assert_eq!(
            rows(&page)[0].field,
            Field::Number {
                integer: true,
                value: "2".into()
            }
        );
    }

    #[test]
    fn aliases_match_the_table_row() {
        let mut uv = spec("uv", json!(["point2"]), Value::Null, "none");
        uv["aliases"] = json!(["st"]);
        let page = json!({"parameters":[declared("st","point2","floats",json!([0,0,1,0]))],
                          "schema":[uv]});
        let rows = rows(&page);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "st");
        assert!(rows[0].declared && rows[0].known);
    }

    #[test]
    fn edits_declare_replace_and_reset() {
        let target = json!({"kind":"resource","id":"r"});
        let page = json!({
            "parameters":[{"name":"reflectance","value_type":"rgb",
                "color_space":{"kind":"explicit","value":"aces2065-1"},
                "values":{"kind":"floats","value":[0.5,0.5,0.5]},"preview_truncated":false}],
            "schema":[spec("roughness",json!(["float"]),json!({"value_type":"float","values":[0.0]}),"constant"),
                      spec("reflectance",json!(["spectrum","rgb"]),Value::Null,"none")],
        });
        let rows = rows(&page);
        let roughness = &rows[0];
        let values = values(&roughness.field, &["0.25".into()]).unwrap();
        assert_eq!(
            set_edit(&target, roughness, values),
            json!({"kind":"set_parameter","target":target,"name":"roughness","value_type":"float",
                   "value":{"name":"roughness","value_type":"float","color_space":null,
                            "values":{"kind":"floats","value":[0.25]}}})
        );
        assert_eq!(reset_edit(&target, roughness), None, "nothing to reset");
        let reflectance = &rows[1];
        // An undeclared color without a default is a greyed zero triple.
        let undeclared = super::rows(&json!({"parameters":[],"schema":[
            spec("reflectance",json!(["spectrum","rgb"]),Value::Null,"none")]}));
        assert_eq!(undeclared[0].field, Field::Rgb([0.0; 3]));
        assert_eq!(undeclared[0].value_type, "rgb");
        assert_eq!(undeclared[0].hint, "not set");
        let values =
            super::values(&reflectance.field, &["1".into(), "0".into(), "0.5".into()]).unwrap();
        let edit = set_edit(&target, reflectance, values);
        assert_eq!(edit["value"]["color_space"], "aces2065-1", "explicit kept");
        assert_eq!(edit["value"]["values"]["value"], json!([1.0, 0.0, 0.5]));
        assert_eq!(
            reset_edit(&target, reflectance).unwrap(),
            json!({"kind":"set_parameter","target":target,"name":"reflectance",
                   "value_type":"rgb","value":null})
        );
        // Bad text does not make an edit.
        assert_eq!(super::values(&roughness.field, &["abc".into()]), None);
        assert_eq!(
            super::values(&reflectance.field, &["1".into(), "x".into(), "0".into()]),
            None
        );
    }

    /// The table's mapping rows group apart from the body, and a row r4
    /// does not read under the current values hides unless declared.
    #[test]
    fn mapping_rows_group_apart_and_hide_when_unread() {
        let mapped = |name: &str, default: Value, when: Value| {
            let mut row = spec(name, json!(["float"]), default, "constant");
            row["group"] = json!("mapping");
            row["when"] = when;
            row
        };
        let float = |value: f64| json!({"value_type":"float","values":[value]});
        let mut mapping = mapped(
            "mapping",
            json!({"value_type":"string","values":["uv"]}),
            json!([]),
        );
        mapping["value_types"] = json!(["string"]);
        let schema = json!([
            spec(
                "dimension",
                json!(["integer"]),
                json!({"value_type":"integer","values":[2]}),
                "constant"
            ),
            spec("scale", json!(["float"]), float(1.), "constant"),
            mapping,
            mapped(
                "uscale",
                float(1.),
                json!([{"parameter":"dimension","values":["2"]},{"parameter":"mapping","values":["uv"]}])
            ),
            mapped(
                "udelta",
                float(0.),
                json!([{"parameter":"mapping","values":["uv","planar"]}])
            ),
            mapped(
                "v1",
                float(0.),
                json!([{"parameter":"mapping","values":["planar"]}])
            ),
        ]);
        let shown = |parameters: Value| {
            let rows = rows(&json!({"parameters":parameters,"schema":schema}));
            let names = |mapping: bool| {
                rows.iter()
                    .filter(|row| (row.group.as_deref() == Some("mapping")) == mapping)
                    .filter(|row| in_use(&rows, row))
                    .map(|row| row.name.clone())
                    .collect::<Vec<_>>()
            };
            (names(false), names(true))
        };
        // Undeclared mapping means uv.
        assert_eq!(
            shown(json!([])),
            (
                vec!["dimension".into(), "scale".into()],
                vec!["mapping".into(), "uscale".into(), "udelta".into()]
            )
        );
        let planar = declared("mapping", "string", "strings", json!(["planar"]));
        assert_eq!(
            shown(json!([planar])).1,
            ["mapping", "udelta", "v1"],
            "planar reads v1 and the deltas, not the scales"
        );
        let uscale = declared("uscale", "float", "floats", json!([2.0]));
        assert_eq!(
            shown(json!([planar, uscale])).1,
            ["mapping", "uscale", "udelta", "v1"],
            "a declared row stays visible even when unread"
        );
        let spherical = declared("mapping", "string", "strings", json!(["spherical"]));
        assert_eq!(shown(json!([spherical])).1, ["mapping"]);
        let three = declared("dimension", "integer", "integers", json!(["3"]));
        assert_eq!(shown(json!([three])).1, ["mapping", "udelta"]);
    }

    /// A single geometric tuple (the table gives it a three-value default)
    /// edits as x / y / z, declared or not; an array row (a mesh's N, no
    /// default) or an unknown row keeps JSON even when it holds three values.
    #[test]
    fn vectors_edit_as_three_numbers() {
        let tuple = |name: &str, value_type: &str| {
            spec(
                name,
                json!([value_type]),
                json!({"value_type":value_type,"values":[0.0, 0.0, 1.0]}),
                "constant",
            )
        };
        for value_type in ["vector", "vector3", "point3", "normal"] {
            let page = json!({
                "parameters":[declared("v1", value_type, "floats", json!([1.0, 0.0, 0.5]))],
                "schema":[tuple("v1", value_type)]});
            assert_eq!(
                rows(&page)[0].field,
                Field::Vector([1.0, 0.0, 0.5]),
                "{value_type}"
            );
        }
        let one_normal = declared("N", "normal", "floats", json!([0.0, 0.0, 1.0]));
        let mesh = json!({"parameters":[one_normal.clone()],
            "schema":[spec("N", json!(["normal"]), Value::Null, "none")]});
        assert_eq!(rows(&mesh)[0].field, Field::Json, "an array row stays JSON");
        let unknown = json!({"parameters":[declared("extra", "vector3", "floats", json!([1.0, 2.0, 3.0]))],
            "schema":[]});
        assert_eq!(
            rows(&unknown)[0].field,
            Field::Json,
            "an unknown row stays JSON"
        );
        let page = json!({"parameters":[],"schema":[spec("v2", json!(["vector"]),
            json!({"value_type":"vector","values":[0.0, 1.0, 0.0]}), "constant")]});
        let row = rows(&page).remove(0);
        assert_eq!(row.field, Field::Vector([0.0, 1.0, 0.0]));
        assert_eq!(row.value_type, "vector");
        assert_eq!(
            values(&row.field, &["1".into(), "2".into(), "-3".into()]),
            Some(json!({"kind":"floats","value":[1.0, 2.0, -3.0]}))
        );
        assert_eq!(
            values(&row.field, &["1".into(), "x".into(), "3".into()]),
            None
        );
    }

    /// A row with fixed choices carries them; a value outside them (from an
    /// import or the CLI) cannot say which rows r4 reads, so they all show.
    #[test]
    fn unknown_choices_show_every_dependent_row() {
        let mut mapping = spec(
            "mapping",
            json!(["string"]),
            json!({"value_type":"string","values":["uv"]}),
            "constant",
        );
        mapping["choices"] = json!(["uv", "spherical", "cylindrical", "planar"]);
        let mut v1 = spec(
            "v1",
            json!(["float"]),
            json!({"value_type":"float","values":[0.0]}),
            "constant",
        );
        v1["when"] = json!([{"parameter":"mapping","values":["planar"]}]);
        let page = |value: &str| {
            json!({
            "parameters":[declared("mapping", "string", "strings", json!([value]))],
            "schema":[mapping.clone(), v1.clone()]})
        };
        let rows_for = |value: &str| rows(&page(value));
        assert_eq!(
            rows_for("uv")[0].choices,
            ["uv", "spherical", "cylindrical", "planar"]
        );
        assert!(rows_for("uv")[1].choices.is_empty());
        // The read-rows rule itself:
        // v1 is unread under uv, read under planar, and an unknown mode
        // hides nothing.
        let read = |value: &str| {
            let found = rows_for(value);
            found.iter().filter(|row| in_use(&found, row)).count()
        };
        assert_eq!(read("uv"), 1, "v1 is unread under uv");
        assert_eq!(read("planar"), 2);
        assert_eq!(read("bogus"), 2, "an unknown mode hides nothing");
    }

    /// Suggestions ride with free text; an empty choice reads "(none)".
    #[test]
    fn suggestions_and_the_empty_choice_have_labels() {
        let mut encoding = spec("encoding", json!(["string"]), Value::Null, "depends");
        encoding["suggestions"] = json!(["sRGB", "linear", "gamma 2.2"]);
        let row = rows(&json!({"parameters":[],"schema":[encoding]})).remove(0);
        assert_eq!(row.suggestions, ["sRGB", "linear", "gamma 2.2"]);
        assert!(row.choices.is_empty());
        assert_eq!(row.field, Field::Text(String::new()));
        assert_eq!(choice_label(""), "(none)");
        assert_eq!(choice_label("Skimmilk"), "Skimmilk");
    }

    #[test]
    fn spectrum_data_references_map_to_a_field_not_json() {
        let field_of = |p: Value| field(&p, false);
        // A Resources{spectrum_data} reference is a field of its own —
        // never the JSON fallback.
        let reference = json!({"name":"k","value_type":"spectrum",
            "values":{"kind":"resources","resource_kind":"spectrum_data","ids":["sd-1"]},
            "preview_truncated":false});
        assert_eq!(field_of(reference), Field::SpectrumData("sd-1".into()));
        // The other resource kinds keep their fields.
        let texture = json!({"name":"reflectance","value_type":"texture",
            "values":{"kind":"resources","resource_kind":"texture","ids":["t"]}});
        assert_eq!(field_of(texture), Field::Texture("t".into()));
        let materials = json!({"name":"materials","value_type":"string",
            "values":{"kind":"resources","resource_kind":"material","ids":["a"]}});
        assert_eq!(field_of(materials), Field::Materials(vec!["a".into()]));
    }

    #[test]
    fn mix_material_edits_fill_or_replace_one_operand() {
        let target = json!({"kind":"resource","id":"mix"});
        assert_eq!(
            mix_material_edit(&target, &[], 0, "a"),
            json!({
                "kind":"set_parameter","target":target,"name":"materials","value_type":"string",
                "value":{"name":"materials","value_type":"string","color_space":null,
                    "values":{"kind":"resources","value":{"kind":"material","ids":["a","a"]}}}
            })
        );
        assert_eq!(
            mix_material_edit(&target, &["a".into(), "b".into()], 1, "c")["value"]["values"]["value"]
                ["ids"],
            json!(["a", "c"])
        );
        // The wire must take it.
        let mix = json!({"kind":"resource","id":"00000000-0000-4000-8000-000000000001"});
        serde_json::from_value::<protocol::project::ProjectEdit>(mix_material_edit(
            &mix,
            &[],
            0,
            "00000000-0000-4000-8000-000000000002",
        ))
        .expect("mix edit parses as a ProjectEdit");
    }

    #[test]
    fn row_disconnect_menu_only_offers_optional_connected_texture_rows() {
        let target = json!({"kind":"resource","id":"wall"});
        let edit = json!({"kind":"disconnect_texture","target":target,"parameter":"albedo"});
        assert_eq!(
            row_disconnect_menu_edit(&target, "albedo", false, true, false, true, Some("string")),
            Some(edit)
        );
        assert_eq!(
            row_disconnect_menu_edit(&target, "albedo", false, false, false, true, Some("string")),
            None
        );
        assert_eq!(
            row_disconnect_menu_edit(
                &target,
                "materials",
                true,
                true,
                false,
                false,
                Some("string")
            ),
            None
        );
        assert_eq!(
            row_disconnect_menu_edit(&target, "albedo", false, true, true, true, Some("string")),
            None
        );
        assert_eq!(
            row_disconnect_menu_edit(&target, "k", false, true, false, false, Some("spectrum")),
            Some(json!({"kind":"set_parameter","target":target,"name":"k",
                "value_type":"spectrum","value":null}))
        );
    }

    #[test]
    fn node_disconnect_menu_lists_outgoing_editable_edges_and_skips_required_slots() {
        let graph = json!({
            "nodes":[
                {"id":"tint","kind":"texture"},
                {"id":"wall","label":"Wall","inputs":[
                    {"parameter":"reflectance","connectable":true,"required":false}]},
                {"id":"mix","label":"Mix","inputs":[
                    {"parameter":"materials","accepts":"material","connectable":false,"required":true}]}
            ],
            "edges":[
                {"from":"tint","to":"wall","parameter":"reflectance"},
                {"from":"tint","to":"mix","parameter":"materials","index":1}
            ]
        });
        let actions = node_disconnect_menu(&graph, "tint", false);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].label, "Disconnect from Wall · reflectance");
        assert_eq!(actions[0].displaced, "tint");
        assert_eq!(
            actions[0].edit,
            json!({"kind":"disconnect_texture",
            "target":{"kind":"resource","id":"wall"},"parameter":"reflectance"})
        );
        assert!(node_disconnect_menu(&graph, "tint", true).is_empty());
        assert!(!node_delete_allowed("root", "root", false));
        assert!(!node_delete_allowed("tint", "root", true));
        assert!(node_delete_allowed("tint", "root", false));
    }

    /// The schema's value types ride the row, and a spectrum-capable
    /// row (its value_types hold `spectrum`) shows one colour box — or a
    /// Every spectrum-capable row shows one colour box (an unbounded
    /// row the chromaticity box now, not a curve sample), and each row
    /// names the role its swatch asks for.
    #[test]
    fn spectrum_rows_show_one_colour_box_and_pick_their_role() {
        let mut reflectance = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        reflectance["texture"] = json!({"kind":"spectrum_albedo"});
        let mut eta = spec(
            "eta",
            json!(["spectrum", "rgb", "float"]),
            json!({"value_type":"spectrum","values":["metal-Cu-eta"]}),
            "constant",
        );
        eta["texture"] = json!({"kind":"spectrum_unbounded"});
        let light = spec(
            "I",
            json!(["spectrum", "rgb", "blackbody"]),
            json!(null),
            "illuminant",
        );
        let mut tex1 = spec("tex1", json!(["spectrum", "rgb"]), Value::Null, "none");
        tex1["texture"] = json!({"kind":"spectrum_albedo"});
        let page = json!({"parameters":[], "schema":[
            reflectance, eta, light, tex1,
            spec("roughness",json!(["float"]),json!({"value_type":"float","values":[0.0]}),"constant")]});
        let rows = rows(&page);
        // The schema's value types and the raw texture kind ride the row.
        assert_eq!(rows[0].value_types, ["spectrum", "rgb"]);
        assert_eq!(rows[0].texture_kind.as_deref(), Some("spectrum_albedo"));
        assert_eq!(rows[1].texture_kind.as_deref(), Some("spectrum_unbounded"));
        assert_eq!(
            rows[2].value_types,
            ["spectrum", "rgb", "blackbody"],
            "the light's I has no texture but its value types ride anyway"
        );
        // Spectrum-capable rows all take the colour box, unbounded
        // included.
        assert_eq!(spectrum_view(&rows[0]), SpectrumView::Colour);
        assert_eq!(spectrum_view(&rows[1]), SpectrumView::Colour);
        assert_eq!(spectrum_view(&rows[2]), SpectrumView::Colour);
        assert_eq!(spectrum_view(&rows[3]), SpectrumView::Colour);
        // Not spectrum-capable: the typed field stays.
        assert_eq!(spectrum_view(&rows[4]), SpectrumView::Typed);
        // A number on a spectrum-capable slot (dielectric `float eta`,
        // declared or as the default) keeps its number field.
        let dielectric_eta = spec(
            "eta",
            json!(["float", "spectrum"]),
            json!({"value_type":"float","values":[1.5]}),
            "constant",
        );
        let default_eta =
            &super::rows(&json!({"parameters":[], "schema":[dielectric_eta.clone()]}))[0];
        assert_eq!(spectrum_view(default_eta), SpectrumView::Typed);
        let declared_eta = &super::rows(&json!({"parameters":[{"name":"eta","value_type":"float",
            "values":{"kind":"floats","value":[1.6]},"preview_truncated":false}],
            "schema":[dielectric_eta]}))[0];
        assert!(declared_eta.declared);
        assert_eq!(spectrum_view(declared_eta), SpectrumView::Typed);
        // The role each row's swatch asks — reflectances keep the
        // albedo read, the lights' colour rows show the chromaticity, an
        // unbounded eta does too, and rows without a table preference
        // leave the choice to the core's schema role.
        assert_eq!(row_role(&rows[0]).as_deref(), Some("albedo"));
        assert_eq!(row_role(&rows[1]).as_deref(), Some("chromaticity"));
        assert_eq!(row_role(&rows[2]).as_deref(), Some("chromaticity"));
        assert_eq!(row_role(&rows[3]).as_deref(), Some("albedo"));
        assert_eq!(row_role(&rows[4]), None);
        assert_eq!(row_role(default_eta), None, "a typed number asks no colour");
        // The tabs the schema allows: no blackbody for reflectance or the
        // conductor's operands, all three for the light.
        assert_eq!(
            spectrum_tabs(&rows[0]),
            SpectrumTabs {
                rgb: true,
                blackbody: false,
                named: true,
                spectrum: true
            }
        );
        assert_eq!(
            spectrum_tabs(&rows[1]),
            SpectrumTabs {
                rgb: true,
                blackbody: false,
                named: true,
                spectrum: true
            }
        );
        assert_eq!(
            spectrum_tabs(&rows[2]),
            SpectrumTabs {
                rgb: true,
                blackbody: true,
                named: true,
                spectrum: true
            }
        );
    }

    /// A spectrum-capable row that is not in the table (declared by an
    /// import or the CLI) keeps its typed field, and rows surfaced with no
    /// schema keep the JSON fallback.
    #[test]
    fn rows_without_a_table_entry_are_never_spectrum_capable() {
        let pairs = json!({
            "parameters":[declared("L", "spectrum", "floats", json!([400., 0.5, 700., 0.5]))],
            "schema":[]});
        let row = &rows(&pairs)[0];
        assert!(row.declared && !row.known);
        assert_eq!(row.field, Field::Json);
        assert!(row.value_types.is_empty());
        assert_eq!(spectrum_view(row), SpectrumView::Typed);
        assert_eq!(
            spectrum_tabs(row),
            SpectrumTabs {
                rgb: false,
                blackbody: false,
                named: false,
                spectrum: false
            }
        );
    }

    /// The dialog opens on the tab of the value the row holds — the
    /// declared value's spectral kind, or the greyed default's.
    #[test]
    fn the_row_names_the_dialog_tab_its_value_belongs_to() {
        let page = |parameter: Option<Value>, schema: Value| json!({"parameters":parameter.into_iter().collect::<Vec<_>>(), "schema":[schema]});
        let mut reflectance = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        reflectance["texture"] = json!({"kind":"spectrum_albedo"});
        let grid = |declared: Value| rows(&page(Some(declared), reflectance.clone())).remove(0);
        assert_eq!(
            current_tab(&grid(declared(
                "reflectance",
                "rgb",
                "floats",
                json!([0.1, 0.2, 0.3])
            ))),
            SpectrumTab::Rgb
        );
        assert_eq!(
            current_tab(&grid(declared(
                "reflectance",
                "blackbody",
                "floats",
                json!([6500.])
            ))),
            SpectrumTab::Blackbody
        );
        assert_eq!(
            current_tab(&grid(declared(
                "reflectance",
                "spectrum",
                "builtin_spectra",
                json!(["metal-Cu-eta"])
            ))),
            SpectrumTab::Named
        );
        assert_eq!(
            current_tab(&grid(declared(
                "reflectance",
                "spectrum",
                "floats",
                json!([400., 0.5, 700., 0.5])
            ))),
            SpectrumTab::Spectrum,
            "inline λ/value pairs sit in the Spectrum Data tab"
        );
        assert_eq!(
            current_tab(&grid(json!({"name":"reflectance","value_type":"spectrum",
                "values":{"kind":"resources","resource_kind":"spectrum_data","ids":["sd-1"]},
                "preview_truncated":false}))),
            SpectrumTab::Spectrum
        );
        assert_eq!(
            current_tab(&grid(json!({"name":"reflectance","value_type":"spectrum",
                "values":{"kind":"assets","value":["spd"]},"preview_truncated":false}))),
            SpectrumTab::Spectrum,
            "a legacy spd file sits in the Spectrum tab"
        );
        // An unset row's greyed default names its tab.
        let unset = &rows(&page(None, reflectance))[0];
        assert!(!unset.declared);
        assert_eq!(current_tab(unset), SpectrumTab::Rgb);
        let mut eta = spec(
            "eta",
            json!(["spectrum", "rgb", "float"]),
            json!({"value_type":"spectrum","values":["metal-Cu-eta"]}),
            "constant",
        );
        eta["texture"] = json!({"kind":"spectrum_unbounded"});
        let eta_row = &rows(&page(None, eta))[0];
        assert_eq!(eta_row.field, Field::Spectrum("metal-Cu-eta".into()));
        assert_eq!(current_tab(eta_row), SpectrumTab::Named);
        // A greyed scalar on a schema that takes no rgb (dielectric's
        // eta): the Spectrum tab is the one it offers — the scalar
        // field lives there, while the wheel would show a body the
        // tab rule hides.
        let glass_eta = spec(
            "eta",
            json!(["float", "spectrum"]),
            json!({"value_type":"float","values":[1.5]}),
            "constant",
        );
        let glass_row = &rows(&page(None, glass_eta))[0];
        assert!(!glass_row.declared);
        assert_eq!(
            glass_row.field,
            Field::Number {
                integer: false,
                value: "1.5".into()
            }
        );
        assert!(!spectrum_tabs(glass_row).rgb, "the dielectric forbids rgb");
        assert_eq!(
            current_tab(glass_row),
            SpectrumTab::Spectrum,
            "the greyed scalar sits where its number field renders"
        );
        // A schema that allows rgb keeps the wheel for its greyed
        // scalar — the RGB tab is offered, it holds the body.
        let wide_eta = spec(
            "eta",
            json!(["spectrum", "rgb", "float"]),
            json!({"value_type":"float","values":[1.5]}),
            "constant",
        );
        let wide_row = &rows(&page(None, wide_eta))[0];
        assert!(spectrum_tabs(wide_row).rgb);
        assert_eq!(current_tab(wide_row), SpectrumTab::Rgb);
    }

    /// The `project.apply` edits a colour dialog applies. Every edit
    /// deserialises as the real wire type, a type change replaces the
    /// declaration within one transaction (one Undo step), and the tab
    /// defaults are: rgb 1 1 1, blackbody 6500 K, first
    /// SpectrumData else first builtin spectrum.
    #[test]
    fn colour_edits_declare_and_replace_across_types_in_one_transaction() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let mut reflectance = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        reflectance["texture"] = json!({"kind":"spectrum_albedo"});
        let declared_rgb = json!({
            "parameters":[{
                "name":"reflectance","value_type":"rgb",
                "color_space":{"kind":"explicit","value":"aces2065-1"},
                "values":{"kind":"floats","value":[0.25, 0.5, 0.75]},"preview_truncated":false}],
            "schema":[reflectance]});
        let row = &rows(&declared_rgb)[0];
        // A same-type update is one edit, the explicit colour space kept.
        let edits = colour_edits(
            &target,
            row,
            "rgb",
            json!({"kind":"floats","value":[0.5, 0.5, 0.5]}),
        );
        assert_eq!(edits.len(), 1, "{edits:?}");
        assert_eq!(
            edits[0],
            json!({"kind":"set_parameter","target":target,"name":"reflectance","value_type":"rgb",
                    "value":{"name":"reflectance","value_type":"rgb","color_space":"aces2065-1",
                             "values":{"kind":"floats","value":[0.5, 0.5, 0.5]}}})
        );
        // A cross-type change replaces the declaration first — the core
        // refuses to silently change a declared parameter's type — and
        // both edits ride one transaction, so one Undo step returns the
        // rgb value. The colour space keys colour values only: the
        // blackbody declaration drops it.
        let edits = colour_edits(
            &target,
            row,
            "blackbody",
            json!({"kind":"floats","value":[6500.]}),
        );
        assert_eq!(edits.len(), 2, "{edits:?}");
        assert_eq!(
            edits[0],
            json!({"kind":"set_parameter","target":target,"name":"reflectance",
                    "value_type":"rgb","value":null})
        );
        assert_eq!(
            edits[1],
            json!({"kind":"set_parameter","target":target,"name":"reflectance",
                    "value_type":"blackbody",
                    "value":{"name":"reflectance","value_type":"blackbody","color_space":null,
                             "values":{"kind":"floats","value":[6500.]}}})
        );
        // An unset row declares without the removal edit, with no colour
        // space to keep.
        let unset = &rows(&json!({"parameters":[],"schema":[reflectance.clone()]}))[0];
        assert!(!unset.declared);
        let edits = colour_edits(
            &target,
            unset,
            "rgb",
            json!({"kind":"floats","value":[1., 1., 1.]}),
        );
        assert_eq!(edits.len(), 1, "{edits:?}");
        assert_eq!(edits[0]["value"]["color_space"], serde_json::Value::Null);
        // Every payload the dialog builds deserialises as the real wire
        // edit type.
        for edit in &edits {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
    }

    /// The tab defaults' values — rgb 1 1 1, blackbody 6500 K, the
    /// Spectrum tab's first pick among the project's SpectrumData and the
    /// builtin spectra.
    #[test]
    fn tab_defaults_pick_the_planned_values() {
        let target = json!({"kind":"node","id":"00000000-0000-0000-0000-0000000000bb"});
        let mut light = spec(
            "I",
            json!(["spectrum", "rgb", "blackbody"]),
            json!(null),
            "illuminant",
        );
        light["default"] = json!(null);
        let row = &rows(&json!({"parameters":[
            {"name":"I","value_type":"rgb","color_space":{"kind":"inherit"},
             "values":{"kind":"floats","value":[1., 1., 1.]},"preview_truncated":false}],
            "schema":[light]}))[0];
        let edits = tab_default_edits(&target, row, SpectrumTab::Rgb, &json!({}), &[]);
        assert_eq!(
            edits,
            vec![
                json!({"kind":"set_parameter","target":target,"name":"I","value_type":"rgb",
                    "value":{"name":"I","value_type":"rgb","color_space":null,
                             "values":{"kind":"floats","value":[1., 1., 1.]}}})
            ]
        );
        // The Spectrum tab picks the project's first SpectrumData
        // resource, else the first builtin. A declared
        // rgb value replaced by a curve is a type change: the removal
        // rides the same one-transaction apply.
        let items = json!({
            "00000000-0000-0000-0000-00000000ab01":{"kind":"resource","resource_kind":"spectrum_data","name":"b-side.spd"},
            "00000000-0000-0000-0000-00000000ab00":{"kind":"resource","resource_kind":"spectrum_data","name":"a-side.spd"},
            "single":{"kind":"resource","resource_kind":"material","name":"albedo"},
        });
        let builtins = vec!["glass-BK7".to_owned()];
        let removal = json!({"kind":"set_parameter","target":target,"name":"I","value_type":"rgb",
                             "value":null});
        let spectrum_data = json!({"kind":"set_parameter","target":target,"name":"I","value_type":"spectrum",
                    "value":{"name":"I","value_type":"spectrum","color_space":null,
                             "values":{"kind":"resources","value":{"kind":"spectrum_data",
                                                                    "ids":["00000000-0000-0000-0000-00000000ab00"]}}}});
        assert_eq!(
            tab_default_edits(&target, row, SpectrumTab::Spectrum, &items, &builtins),
            vec![removal, spectrum_data]
        );
        // No SpectrumData in the project: the Spectrum Data tab opens
        // without an edit; Named takes the first
        // builtin spectrum.
        assert!(
            tab_default_edits(&target, row, SpectrumTab::Spectrum, &json!({}), &builtins)
                .is_empty()
        );
        let builtin = json!({"kind":"set_parameter","target":target,"name":"I","value_type":"spectrum",
                    "value":{"name":"I","value_type":"spectrum","color_space":null,
                             "values":{"kind":"builtin_spectra","value":["glass-BK7"]}}});
        assert_eq!(
            tab_default_edits(&target, row, SpectrumTab::Named, &json!({}), &builtins),
            vec![
                json!({"kind":"set_parameter","target":target,"name":"I","value_type":"rgb",
                        "value":null}),
                builtin.clone()
            ]
        );
        // The standard white is preferred when the table has it.
        let with_d65 = vec!["canon_eos_100d_b".to_owned(), "stdillum-D65".to_owned()];
        let d65 = tab_default_edits(&target, row, SpectrumTab::Named, &json!({}), &with_d65);
        assert_eq!(
            d65.last().unwrap()["value"]["values"]["value"],
            json!(["stdillum-D65"])
        );
        // Nothing spectral at all: the tab opens without an edit.
        assert!(tab_default_edits(&target, row, SpectrumTab::Named, &json!({}), &[]).is_empty());
        // Every payload the dialog builds deserialises as the real wire
        // edit type.
        for edit in tab_default_edits(&target, row, SpectrumTab::Named, &items, &builtins)
            .into_iter()
            .chain(tab_default_edits(
                &target,
                row,
                SpectrumTab::Spectrum,
                &items,
                &builtins,
            ))
        {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
    }

    /// (items 2–3): the family of named built-in spectra a
    /// parameter row's role picks: a conductor's eta/k are metals, a
    /// (thin)dielectric's eta and a coateddiffuse/coatedconductor
    /// interface.eta are glasses, and a light's colour rows (the
    /// blackbody-capable ones — the lights' I and L) are illuminants.
    /// Anything else fits no family: the list stays full and the
    /// default today's. A page that cannot name its implementation (a
    /// value-node page without its consuming material) falls back to
    /// the name alone: eta/k are the metals' operands.
    #[test]
    fn named_families_follow_the_rows_role() {
        let curve = |name: &str, default: &str| {
            json!({"name":name,"aliases":[],"value_types":["spectrum","rgb","float"],
                   "default":{"value_type":"spectrum","values":[default]},
                   "default_kind":"constant","default_depends":null,
                   "texture":{"kind":"spectrum_unbounded"},"role":"texture_input",
                   "required":false})
        };
        let row_of = |implementation: &str, schema: Value| {
            rows(&json!({"parameters":[], "schema":[schema], "implementation":implementation}))
                .remove(0)
        };
        // The metals: conductor eta/k, coatedconductor conductor.eta/k.
        assert_eq!(
            named_family(&row_of("conductor", curve("eta", "metal-Cu-eta"))),
            Some(NamedFamily::MetalEta)
        );
        assert_eq!(
            named_family(&row_of("conductor", curve("k", "metal-Cu-k"))),
            Some(NamedFamily::MetalK)
        );
        assert_eq!(
            named_family(&row_of(
                "coatedconductor",
                curve("conductor.eta", "metal-Cu-eta")
            )),
            Some(NamedFamily::MetalEta)
        );
        assert_eq!(
            named_family(&row_of(
                "coatedconductor",
                curve("conductor.k", "metal-Cu-k")
            )),
            Some(NamedFamily::MetalK)
        );
        // The glasses:dielectric eta, coateddiffuse and
        // coatedconductor interface.eta.
        assert_eq!(
            named_family(&row_of("dielectric", curve("eta", "glass-BK7"))),
            Some(NamedFamily::Glass)
        );
        assert_eq!(
            named_family(&row_of("thindielectric", curve("eta", "glass-BK7"))),
            Some(NamedFamily::Glass)
        );
        assert_eq!(
            named_family(&row_of(
                "coateddiffuse",
                curve("interface.eta", "glass-BK7")
            )),
            Some(NamedFamily::Glass)
        );
        assert_eq!(
            named_family(&row_of(
                "coatedconductor",
                curve("interface.eta", "glass-BK7")
            )),
            Some(NamedFamily::Glass)
        );
        // The lights' colour rows: the blackbody heuristic the dialog
        // keys on names them, whatever the light's implementation.
        let light = json!({"name":"L","aliases":[],"value_types":["spectrum","rgb","blackbody"],
                          "default":null,"default_kind":"illuminant","default_depends":null,
                          "texture":null,"role":"texture_input","required":false});
        assert_eq!(
            named_family(&row_of("diffuse", light)),
            Some(NamedFamily::Illuminant)
        );
        // A plain reflectance fits no family.
        let mut reflectance = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        reflectance["texture"] = json!({"kind":"spectrum_albedo"});
        assert_eq!(named_family(&row_of("diffuse", reflectance)), None);
        // Without an implementation at all, eta/k still name the
        // metals' operands — everything else fits no family.
        assert_eq!(
            named_family(&row_of("", curve("eta", "metal-Cu-eta"))),
            Some(NamedFamily::MetalEta)
        );
        assert_eq!(
            named_family(&row_of("", curve("k", "metal-Cu-k"))),
            Some(NamedFamily::MetalK)
        );
        let mut albedo = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        albedo["texture"] = json!({"kind":"spectrum_albedo"});
        assert_eq!(
            named_family(&rows(&json!({"parameters":[], "schema":[albedo]}))[0]),
            None
        );
    }

    /// (items 2–3): which built-in names belong to a family, and
    /// the default a family's rows name: its canonical curve when the
    /// table has it, else its first member — a table without the family
    /// falls back to today's default, as no-family rows always do.
    #[test]
    fn named_families_match_their_members_and_default_to_their_own() {
        // Membership: the metals split by their operand, the glasses and
        // the illuminants stay one set, the camera curves belong to none.
        assert!(NamedFamily::MetalEta.contains("metal-Cu-eta"));
        assert!(NamedFamily::MetalEta.contains("metal-CuZn-eta"));
        assert!(!NamedFamily::MetalEta.contains("metal-Cu-k"));
        assert!(!NamedFamily::MetalEta.contains("glass-BK7"));
        assert!(NamedFamily::MetalK.contains("metal-Cu-k"));
        assert!(!NamedFamily::MetalK.contains("metal-Cu-eta"));
        assert!(NamedFamily::Glass.contains("glass-BK7"));
        assert!(NamedFamily::Glass.contains("glass-LASF9"));
        assert!(!NamedFamily::Glass.contains("metal-Al-eta"));
        assert!(NamedFamily::Illuminant.contains("stdillum-D65"));
        assert!(NamedFamily::Illuminant.contains("stdillum-A"));
        assert!(NamedFamily::Illuminant.contains("illum-acesD60"));
        assert!(!NamedFamily::Illuminant.contains("canon_eos_5d_r"));
        for family in [
            NamedFamily::MetalEta,
            NamedFamily::MetalK,
            NamedFamily::Glass,
            NamedFamily::Illuminant,
        ] {
            assert!(!family.contains("canon_eos_5d_mkiv_r"));
        }
        // The defaults, over the table's own order (sorted, as r4
        // serves them).
        let builtins = [
            "canon_eos_5d_r",
            "glass-BK7",
            "illum-acesD60",
            "metal-Ag-eta",
            "metal-Ag-k",
            "metal-Cu-eta",
            "metal-Cu-k",
            "stdillum-A",
            "stdillum-D65",
        ]
        .map(str::to_owned);
        assert_eq!(
            NamedFamily::MetalEta
                .default_name(&builtins)
                .map(String::as_str),
            Some("metal-Cu-eta")
        );
        assert_eq!(
            NamedFamily::MetalK
                .default_name(&builtins)
                .map(String::as_str),
            Some("metal-Cu-k")
        );
        assert_eq!(
            NamedFamily::Glass
                .default_name(&builtins)
                .map(String::as_str),
            Some("glass-BK7")
        );
        assert_eq!(
            NamedFamily::Illuminant
                .default_name(&builtins)
                .map(String::as_str),
            Some("stdillum-D65")
        );
        // The canonical name missing: the family's first member.
        let no_copper = &builtins[..5];
        assert_eq!(
            NamedFamily::MetalEta
                .default_name(no_copper)
                .map(String::as_str),
            Some("metal-Ag-eta")
        );
        assert_eq!(
            NamedFamily::MetalK
                .default_name(no_copper)
                .map(String::as_str),
            Some("metal-Ag-k")
        );
        // A table without the family at all: no name of the family's —
        // the caller falls back to today's default.
        let no_metals = &builtins[..2];
        assert_eq!(NamedFamily::MetalEta.default_name(no_metals), None);
    }

    /// The Named tab's switch declares the family's own
    /// default — the conductor's eta takes copper, its k copper's k,
    /// the lights and the no-family rows keep the standard white.
    #[test]
    fn tab_defaults_name_the_familys_own_spectrum() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let curve = |name: &str, default: &str| {
            json!({"name":name,"aliases":[],"value_types":["spectrum","rgb","float"],
                   "default":{"value_type":"spectrum","values":[default]},
                   "default_kind":"constant","default_depends":null,
                   "texture":{"kind":"spectrum_unbounded"},"role":"texture_input",
                   "required":false})
        };
        let builtins = ["metal-Cu-eta", "metal-Cu-k", "stdillum-D65"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        let sets = |row: &Row, name: &str| {
            vec![
                json!({"kind":"set_parameter","target":target,"name":row.name,
                    "value_type":"spectrum",
                    "value":{"name":row.name,"value_type":"spectrum","color_space":null,
                             "values":{"kind":"builtin_spectra","value":[name]}}}),
            ]
        };
        let eta = &rows(
            &json!({"parameters":[], "schema":[curve("eta", "metal-Cu-eta")],
                                "implementation":"conductor"}),
        )[0];
        assert_eq!(
            tab_default_edits(&target, eta, SpectrumTab::Named, &json!({}), &builtins),
            sets(eta, "metal-Cu-eta")
        );
        let k = &rows(
            &json!({"parameters":[], "schema":[curve("k", "metal-Cu-k")],
                              "implementation":"conductor"}),
        )[0];
        assert_eq!(
            tab_default_edits(&target, k, SpectrumTab::Named, &json!({}), &builtins),
            sets(k, "metal-Cu-k")
        );
        // The light's colour row and a no-family row keep today's
        // default: the standard white.
        let light = json!({"name":"L","aliases":[],"value_types":["spectrum","rgb","blackbody"],
                          "default":null,"default_kind":"illuminant","default_depends":null,
                          "texture":null,"role":"texture_input","required":false});
        let l = &rows(&json!({"parameters":[], "schema":[light], "implementation":"diffuse"}))[0];
        assert_eq!(
            tab_default_edits(&target, l, SpectrumTab::Named, &json!({}), &builtins),
            sets(l, "stdillum-D65")
        );
        let mut reflectance = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        reflectance["texture"] = json!({"kind":"spectrum_albedo"});
        let albedo = &rows(&json!({"parameters":[], "schema":[reflectance],
                                   "implementation":"diffuse"}))[0];
        assert_eq!(
            tab_default_edits(&target, albedo, SpectrumTab::Named, &json!({}), &builtins),
            sets(albedo, "stdillum-D65")
        );
        for edits in [&sets(eta, "metal-Cu-eta"), &sets(k, "metal-Cu-k")] {
            for edit in edits {
                serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                    .unwrap_or_else(|error| panic!("{error}: {edit}"));
            }
        }
    }

    /// The light's `scale` rides in the colour dialog,
    /// shown alongside — only a page with a blackbody-capable colour row that also
    /// declares a plain constant `scale` gets one; a mix's `scale` operand
    /// (a texture input, no blackbody tab anywhere) does not.
    #[test]
    fn the_dialog_shows_a_lights_scale_but_not_a_mix_textures() {
        let light = spec(
            "I",
            json!(["spectrum", "rgb", "blackbody"]),
            Value::Null,
            "illuminant",
        );
        let scale = spec(
            "scale",
            json!(["float"]),
            json!({"value_type":"float","values":[1.]}),
            "constant",
        );
        let row = dialog_scale(&rows(&json!({"parameters":[],"schema":[light, scale]})));
        assert_eq!(row.as_ref().map(|row| row.name.as_str()), Some("scale"));
        assert!(
            dialog_scale(&rows(&json!({"parameters":[],"schema":[light]}))).is_none(),
            "no scale row, no block"
        );
        let mut mix_scale = spec(
            "scale",
            json!(["float"]),
            json!({"value_type":"float","values":[1.]}),
            "constant",
        );
        mix_scale["texture"] = json!({"kind":"float"});
        let amount = spec(
            "amount",
            json!(["float"]),
            json!({"value_type":"float","values":[0.5]}),
            "constant",
        );
        assert!(
            dialog_scale(&rows(
                &json!({"parameters":[],"schema":[mix_scale, amount]})
            ))
            .is_none(),
            "a mix texture has no blackbody colour row: its scale never shows"
        );
        // Even a light page hides a scale that takes a texture (none of
        // the tables spells one today; the guard keeps the shown alongside honest).
        let mut texture_scale = scale;
        texture_scale["texture"] = json!({"kind":"float"});
        assert!(
            dialog_scale(&rows(
                &json!({"parameters":[],"schema":[light, texture_scale]})
            ))
            .is_none()
        );
    }

    /// A conductor page's eta/k rows grey out when the metal reads
    /// r4's refused reflectance form instead — the hint says why, and
    /// both declared r4 rejects while reflectance alone says unused.
    #[test]
    fn conductor_eta_and_k_rows_carry_the_unused_hint() {
        let eta = spec(
            "eta",
            json!(["spectrum", "rgb", "float"]),
            json!({"value_type":"spectrum","values":["metal-Cu-eta"]}),
            "constant",
        );
        let k = spec(
            "k",
            json!(["spectrum", "rgb", "float"]),
            json!({"value_type":"spectrum","values":["metal-Cu-k"]}),
            "constant",
        );
        let reflectance = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        let roughness = spec(
            "roughness",
            json!(["float"]),
            json!({"value_type":"float","values":[0.0]}),
            "constant",
        );
        let page = |implementation: &str, parameters: Value| json!({"implementation":implementation, "parameters":parameters, "schema":[eta, k, reflectance, roughness]});
        // Without a reflectance, r4 reads eta and k normally.
        let bare = &rows(&page(
            "conductor",
            json!([declared(
                "k",
                "spectrum",
                "builtin_spectra",
                json!(["metal-Au-k"])
            )]),
        ))[0];
        assert_eq!(bare.ignored, None);
        assert_eq!(rows(&page("conductor", json!([])))[1].ignored, None);
        // Only a reflectance: eta and k are unused.
        let only_reflectance = rows(&page(
            "conductor",
            json!([declared(
                "reflectance",
                "rgb",
                "floats",
                json!([0.8, 0.6, 0.2])
            )]),
        ));
        assert_eq!(only_reflectance[0].ignored, Some(UNUSED_REFLECTANCE));
        assert_eq!(only_reflectance[1].ignored, Some(UNUSED_REFLECTANCE));
        assert_eq!(
            only_reflectance[2].ignored, None,
            "the reflectance row itself is in use"
        );
        assert_eq!(only_reflectance[3].ignored, None, "roughness stays in use");
        // A reflectance beside eta/k: r4 refuses the pair.
        let conflict = rows(&page(
            "conductor",
            json!([
                declared("k", "spectrum", "builtin_spectra", json!(["metal-Au-k"])),
                declared("reflectance", "rgb", "floats", json!([0.8, 0.6, 0.2]))
            ]),
        ));
        assert_eq!(
            conflict[0].ignored,
            Some("r4 rejects reflectance together with eta/k")
        );
        assert_eq!(
            conflict[1].ignored,
            Some("r4 rejects reflectance together with eta/k")
        );
        // A coatedconductor page marks its prefixed rows the same way.
        let prefixed_eta = spec(
            "conductor.eta",
            json!(["spectrum", "rgb", "float"]),
            json!({"value_type":"spectrum","values":["metal-Cu-eta"]}),
            "constant",
        );
        let prefixed_k = spec(
            "conductor.k",
            json!(["spectrum", "rgb", "float"]),
            json!({"value_type":"spectrum","values":["metal-Cu-k"]}),
            "constant",
        );
        let coated = rows(&json!({"implementation":"coatedconductor",
            "parameters":[declared("reflectance","rgb","floats",json!([0.9, 0.5, 0.1]))],
            "schema":[prefixed_eta, prefixed_k, reflectance]}));
        assert_eq!(coated[0].ignored, Some(UNUSED_REFLECTANCE));
        assert_eq!(coated[1].ignored, Some(UNUSED_REFLECTANCE));
        assert_eq!(coated[2].ignored, None);
        // Not a conductor: eta rows stay plain.
        let mut dielectric_eta = spec(
            "eta",
            json!(["float", "spectrum"]),
            json!({"value_type":"float","values":[1.5]}),
            "constant",
        );
        dielectric_eta["texture"] = json!({"kind":"spectrum_unbounded"});
        let diffuse = rows(&json!({"implementation":"diffuse",
            "parameters":[declared("eta","spectrum","floats",json!([400., 1.]))],
            "schema":[dielectric_eta]}));
        assert_eq!(diffuse[0].ignored, None);
    }

    /// A texture input (a material's color, a shape's alpha) also connects
    /// an image directly: core reuses or creates the imagemap.
    #[test]
    fn texture_inputs_connect_an_image_directly() {
        let mut alpha = spec("alpha", json!(["float"]), Value::Null, "none");
        alpha["texture"] = json!({"kind":"float"});
        let row = rows(&json!({"parameters":[],"schema":[alpha]})).remove(0);
        let items = json!({
            "b": {"kind":"asset","media_type":"image/png","name":"wood.png"},
            "m": {"kind":"asset","media_type":"application/x-ply","name":"toy.ply"},
        });
        let node = json!({"kind":"node","id":"n"});
        assert_eq!(
            connect_image_choices(&node, &row, &items),
            [("wood.png".to_owned(), "b".to_owned())]
        );
        assert_eq!(
            connect_image_edit(&node, &row, "b"),
            json!({"kind":"connect_texture","target":node,"parameter":"alpha","image":"b"})
        );
        // A row that takes no texture offers none.
        let radius = rows(
            &json!({"parameters":[],"schema":[spec("radius", json!(["float"]),
            json!({"value_type":"float","values":[1.0]}), "constant")]}),
        )
        .remove(0);
        assert!(connect_image_choices(&node, &radius, &items).is_empty());
    }

    /// The Connect menu's two sections — the row's textures
    /// and the new-texture items, then the project's Spectrum Data —
    /// each entry the edits it applies; a spectrum-capable row that
    /// takes no texture lists only its Spectra, a plain float row lists
    /// nothing at all. Every payload deserialises as the real wire edit
    /// type.
    #[test]
    fn connect_menu_lists_textures_then_spectrum_data() {
        let mut reflectance = spec(
            "reflectance",
            json!(["spectrum", "rgb"]),
            Value::Null,
            "none",
        );
        reflectance["texture"] = json!({"kind":"spectrum_albedo"});
        let eta = spec(
            "eta",
            json!(["float", "spectrum"]),
            json!({"value_type":"float","values":[1.5]}),
            "constant",
        );
        let roughness = spec(
            "roughness",
            json!(["float"]),
            json!({"value_type":"float","values":[0.0]}),
            "constant",
        );
        let texture_row = rows(&json!({"parameters":[],"schema":[reflectance]})).remove(0);
        let value_row = rows(&json!({"parameters":[],"schema":[eta]})).remove(0);
        let float_row = rows(&json!({"parameters":[],"schema":[roughness]})).remove(0);
        let items = json!({
            "00000000-0000-0000-0000-0000000000c1":{"kind":"resource","resource_kind":"texture","texture_value_type":"spectrum","name":"checks"},
            "00000000-0000-0000-0000-0000000000c2":{"kind":"resource","resource_kind":"texture","texture_value_type":"float","name":"grime"},
            "00000000-0000-0000-0000-00000000ab00":{"kind":"resource","resource_kind":"spectrum_data","name":"a-side.spd"},
            "00000000-0000-0000-0000-00000000ab01":{"kind":"resource","resource_kind":"spectrum_data","name":"b-side.spd"},
            "00000000-0000-0000-0000-0000000000cc":{"kind":"resource","resource_kind":"texture","texture_value_type":"spectrum","name":"me"},
        });
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000cc"});
        // A spectrum texture input: its lineage's textures (never the
        // target itself) and the New items, then the Spectra with the
        // colour dialog's declaration edits.
        let menu = connect_menu(&target, &texture_row, &items, &["marble".to_owned()]);
        assert_eq!(
            menu.textures
                .iter()
                .map(|(label, _)| label.clone())
                .collect::<Vec<_>>(),
            ["checks".to_owned()]
        );
        assert_eq!(
            menu.textures[0].1,
            vec![connect_edit(
                &target,
                &texture_row,
                "00000000-0000-0000-0000-0000000000c1"
            )]
        );
        assert_eq!(menu.create[0].0, "New marble");
        assert_eq!(
            menu.spectra
                .iter()
                .map(|(label, _)| label.clone())
                .collect::<Vec<_>>(),
            ["a-side.spd".to_owned(), "b-side.spd".to_owned()]
        );
        [
            "00000000-0000-0000-0000-00000000ab00",
            "00000000-0000-0000-0000-00000000ab01",
        ]
        .iter()
        .enumerate()
        .for_each(|(index, id)| {
            assert_eq!(
                menu.spectra[index].1,
                colour_edits(
                    &target,
                    &texture_row,
                    "spectrum",
                    json!({"kind":"resources","value":{"kind":"spectrum_data","ids":[id]}})
                )
            );
        });
        // A spectrum-capable value row: only
        // the Spectra — no texture lineage, no New items.
        let menu = connect_menu(&target, &value_row, &items, &[]);
        assert!(menu.textures.is_empty() && menu.create.is_empty());
        assert_eq!(menu.spectra.len(), 2, "only the Spectra are offered");
        // A plain float row: nothing at all — no menu.
        let menu = connect_menu(&target, &float_row, &items, &[]);
        assert!(menu.sections().next().is_none());
        // Every payload the menu builds deserialises as the real wire
        // edit type.
        let menu = connect_menu(&target, &texture_row, &items, &["marble".to_owned()]);
        for edit in menu
            .sections()
            .flat_map(|section| section.iter())
            .flat_map(|(_, edits)| edits.clone())
        {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{error}: {edit}"));
        }
    }

    /// A SpectrumData connection is disconnected by removing the
    /// declaration (set_parameter null) — never disconnect_texture, which
    /// core refuses for a row the table does not call a texture input.
    #[test]
    fn spectrum_data_connections_disconnect_by_dropping_the_declaration() {
        let target = json!({"kind":"resource","id":"00000000-0000-0000-0000-0000000000aa"});
        let connected = json!({"name":"k","value_type":"spectrum",
            "values":{"kind":"resources","resource_kind":"spectrum_data","ids":["sd-1"]},
            "preview_truncated":false});
        let mut k = spec(
            "k",
            json!(["spectrum", "rgb", "float"]),
            Value::Null,
            "none",
        );
        k["texture"] = json!({"kind":"spectrum_unbounded"});
        let row = &rows(&json!({"parameters":[connected],"schema":[k]}))[0];
        assert!(matches!(row.field, Field::SpectrumData(_)));
        let removal = removal_edit(&target, row).unwrap();
        assert_eq!(
            removal,
            json!({"kind":"set_parameter","target":target,"name":"k",
                   "value_type":"spectrum","value":null})
        );
        serde_json::from_value::<protocol::project::ProjectEdit>(removal)
            .expect("the disconnect deserialises as the wire edit type");
    }
}
