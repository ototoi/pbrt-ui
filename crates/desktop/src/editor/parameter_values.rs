//! Array parameter values as Inspector tables (the array parameter
//! review, PR 1): every declared array — inline values of any count, or
//! a data asset above the inline threshold — opens into one table of
//! element groups, read a page at a time through
//! `project.get_parameter_values`. All three storage kinds show the
//! same way. Pure so the layout can be tested without a window;
//! `parameter_field` renders it.
use super::parameter_rows::Row;
use serde_json::{Value, json};

/// Element groups per table page.
pub(super) const PAGE_GROUPS: usize = 100;

/// How one array parameter's table is laid out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Table {
    pub(super) name: String,
    pub(super) value_type: String,
    /// The declared preview's values kind; "array" means a data asset.
    pub(super) kind: String,
    /// Total number of values; groups = ceil.
    pub(super) count: u64,
    /// Values per group.
    pub(super) width: usize,
    /// Column labels after the row-number column.
    pub(super) columns: Vec<String>,
}

impl Table {
    pub(super) fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            name: value["name"].as_str()?.to_owned(),
            value_type: value["value_type"].as_str()?.to_owned(),
            kind: value["kind"].as_str()?.to_owned(),
            count: value["count"].as_u64()?,
            width: value["width"].as_u64()? as usize,
            columns: value["columns"]
                .as_array()?
                .iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
        })
    }

    /// The element groups the table rows show.
    pub(super) fn groups(&self) -> u64 {
        self.count.div_ceil(self.width as u64)
    }

    /// The number of pages the pager offers.
    pub(super) fn pages(&self) -> u64 {
        (self.groups() as usize).div_ceil(PAGE_GROUPS) as u64
    }

    /// The row's value-column text while closed: "point3 · 1681 groups".
    pub(super) fn summary(&self) -> String {
        let kind = if self.value_type.is_empty() {
            self.kind.as_str()
        } else {
            self.value_type.as_str()
        };
        format!("{kind} · {} groups", self.groups())
    }

    /// A kept page index inside the table's page range: the array can
    /// shrink while its row is open.
    pub(super) fn clamped_page(&self, page: usize) -> usize {
        page.min(self.pages().saturating_sub(1) as usize)
    }

    /// The `` window of `page`'s values to ask the core
    /// for. `offset` lands on the page's first whole group, `limit` is
    /// clipped to what is left.
    pub(super) fn window(&self, page: usize) -> (usize, usize) {
        let offset = page * PAGE_GROUPS * self.width;
        let limit = (PAGE_GROUPS * self.width)
            .min((self.count as usize).saturating_sub(offset))
            .max(1);
        (offset, limit)
    }
}

/// The declared type decides how values group into rows. A mesh's
/// `indices` group by its own element, and the implementation says
/// which: a trianglemesh three per triangle, a bilinearmesh four per
/// quad; any other implementation (a PLY's polygons have no fixed
/// width) stays one value per row.
fn layout(name: &str, value_type: &str, implementation: &str) -> (usize, Vec<String>) {
    let columns = |names: &[&str]| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    };
    match (name, value_type) {
        ("indices", "integer") if implementation == "trianglemesh" => {
            (3, columns(&["v0", "v1", "v2"]))
        }
        ("indices", "integer") if implementation == "bilinearmesh" => {
            (4, columns(&["v0", "v1", "v2", "v3"]))
        }
        _ if value_type == "rgb" => (3, columns(&["r", "g", "b"])),
        _ if matches!(
            value_type,
            "point3" | "point" | "vector3" | "vector" | "normal" | "normal3"
        ) =>
        {
            (3, columns(&["x", "y", "z"]))
        }
        _ if matches!(value_type, "point2" | "vector2") => (2, columns(&["x", "y"])),
        _ => (1, columns(&["value"])),
    }
}

/// The array table for a declarable row, or `None` when the row keeps
/// its current field: single values, resource lists
/// and undeclared rows have no values to page through.
pub(super) fn table(row: &Row) -> Option<Table> {
    if !row.declared || !row.parameter.is_object() {
        return None;
    }
    let values = &row.parameter["values"];
    let kind = values["kind"].as_str()?;
    if matches!(kind, "resources") {
        return None;
    }
    // The declared `value_count` is exact for every kind — inline,
    // truncated and data asset alike — before the preview's own count
    // and the few values the preview lists.
    let counted = |value: Option<&Value>| -> Option<u64> {
        let value = value?;
        if let Some(count) = value.as_str().and_then(|count| count.parse::<u64>().ok()) {
            return Some(count);
        }
        value.as_array().map(|list| list.len() as u64)
    };
    let count = counted(row.parameter.get("value_count"))
        .or_else(|| counted(values.get("count")))
        .or_else(|| counted(values.get("value")))?;
    if count < 2 {
        return None;
    }
    let (width, columns) = layout(&row.name, &row.value_type, &row.implementation);
    Some(Table {
        name: row.name.clone(),
        value_type: row.value_type.clone(),
        kind: kind.to_owned(),
        count,
        width,
        columns,
    })
}

/// One cell's text: floats keep the preview's forms,
/// integers stay decimal strings, text kinds show unquoted. A padded
/// `null` reads as a dash.
pub(super) fn cell_text(value: &Value) -> String {
    match value {
        Value::Object(float) => match float.get("kind").and_then(Value::as_str) {
            Some("finite") => float.get("value").map(Value::to_string).unwrap_or_default(),
            Some("positive_infinity") => "inf".into(),
            Some("negative_infinity") => "-inf".into(),
            Some("nan") => "nan".into(),
            _ => value.to_string(),
        },
        Value::String(text) => text.clone(),
        Value::Bool(on) => on.to_string(),
        Value::Null => "—".into(),
        other => other.to_string(),
    }
}

/// One array row's pages as the session keeps them: the project they
/// are current for, and per group page either `None` (asked, not yet
/// answered) or the core's page payload (an `{error}` object when the
/// read failed). A revision change resets every page.
#[derive(Default)]
pub(super) struct ValuesPages {
    at: Value,
    answers: std::collections::BTreeMap<usize, Option<Value>>,
}

impl ValuesPages {
    /// Drop every page: the new project must be asked again.
    pub(super) fn reset(&mut self, at: Value) {
        self.at = at;
        self.answers.clear();
    }

    pub(super) fn current(&self, at: &Value) -> bool {
        self.at == *at
    }

    /// Whether `page` was already queued.
    pub(super) fn asked(&self, page: usize) -> bool {
        self.answers.contains_key(&page)
    }

    /// Queue `page`, keeping any answered payload.
    pub(super) fn mark_asked(&mut self, page: usize) {
        self.answers.entry(page).or_insert(None);
    }

    /// The core's payload for `page` landed; an error becomes `{error}`.
    pub(super) fn answer(&mut self, page: usize, payload: Result<Value, String>) {
        self.answers.insert(
            page,
            Some(payload.unwrap_or_else(|error| json!({ "error": error }))),
        );
    }

    /// The answered payload, or `None` while pending.
    pub(super) fn page(&self, page: usize) -> Option<&Value> {
        self.answers.get(&page).and_then(Option::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::super::parameter_rows::Field;
    use super::*;

    fn finite(value: f64) -> Value {
        json!({"kind":"finite","value":value})
    }

    fn array_row(
        name: &str,
        implementation: &str,
        value_type: &str,
        value_count: &str,
        values: Value,
    ) -> Row {
        Row {
            name: name.into(),
            value_type: value_type.into(),
            implementation: implementation.into(),
            declared: true,
            known: true,
            required: false,
            texture: None,
            texture_kind: None,
            value_types: Vec::new(),
            file: false,
            group: None,
            when: Vec::new(),
            choices: Vec::new(),
            suggestions: Vec::new(),
            field: Field::Json,
            ignored: None,
            hint: String::new(),
            parameter: json!({"name":name,"value_type":value_type,
                // The core truncates every preview above 32 values,
                // data assets included.
                "value_count":value_count,
                "preview_truncated":value_count.parse::<usize>().unwrap_or(0) > 32,
                "values":values}),
        }
    }

    /// The three storage kinds — inline, inline past the
    /// preview cutoff, a data asset — all
    /// make the same table.
    #[test]
    fn the_three_storage_kinds_make_the_same_table() {
        let inline = array_row(
            "P",
            "",
            "point3",
            "12",
            json!({"kind":"floats","value":vec![finite(0.); 12]}),
        );
        let truncated = array_row(
            "P",
            "",
            "point3",
            "363",
            json!({"kind":"floats","value":vec![finite(0.); 32]}),
        );
        let asset = array_row(
            "P",
            "",
            "point3",
            "5043",
            json!({"kind":"array","asset":"a1","encoding":"f64_le","count":"5043"}),
        );
        for (row, kind, count, groups) in [
            (&inline, "floats", 12u64, 4u64),
            (&truncated, "floats", 363, 121),
            (&asset, "array", 5043, 1681),
        ] {
            let layout = table(row).expect("an array row makes a table");
            assert_eq!(
                layout,
                Table {
                    name: "P".into(),
                    value_type: "point3".into(),
                    kind: kind.into(),
                    count,
                    width: 3,
                    columns: vec!["x".into(), "y".into(), "z".into()],
                }
            );
            assert_eq!(layout.groups(), groups);
        }
    }

    /// Groups widen by the declared type, and a trianglemesh's
    /// `indices` rows lay out one triangle per row.
    #[test]
    fn widths_follow_the_type_and_indices_form_triangles() {
        let rows = [
            array_row("uv", "", "point2", "8", json!({"kind":"floats","value":[]})),
            array_row("Kd", "", "rgb", "6", json!({"kind":"floats","value":[]})),
            array_row(
                "indices",
                "trianglemesh",
                "integer",
                "6",
                json!({"kind":"integers","value":[]}),
            ),
            array_row(
                "radii",
                "",
                "float",
                "5",
                json!({"kind":"floats","value":[]}),
            ),
            array_row("N", "", "normal", "9", json!({"kind":"floats","value":[]})),
            array_row("S", "", "vector3", "3", json!({"kind":"floats","value":[]})),
        ];
        let expect = [
            (2usize, vec!["x", "y"], 4u64),
            (3, vec!["r", "g", "b"], 2),
            (3, vec!["v0", "v1", "v2"], 2),
            (1, vec!["value"], 5),
            (3, vec!["x", "y", "z"], 3),
            (3, vec!["x", "y", "z"], 1),
        ];
        for (row, (width, columns, groups)) in rows.iter().zip(expect) {
            let layout = table(row).expect("an array row makes a table");
            assert_eq!(layout.width, width, "{}", row.name);
            assert_eq!(layout.columns, columns, "{}", row.name);
            assert_eq!(layout.groups(), groups, "{}", row.name);
        }
    }

    /// `indices` groups by the owning mesh's element: trianglemeshes one
    /// triangle per row, bilinearmeshes one quad; any other
    /// implementation (a PLY's polygons have no fixed width, an unknown
    /// import says nothing) stays one value per row.
    #[test]
    fn indices_group_by_the_owning_implementation() {
        let rows = [
            array_row(
                "indices",
                "trianglemesh",
                "integer",
                "6",
                json!({"kind":"integers","value":[]}),
            ),
            array_row(
                "indices",
                "bilinearmesh",
                "integer",
                "8",
                json!({"kind":"integers","value":[]}),
            ),
            array_row(
                "indices",
                "plymesh",
                "integer",
                "9",
                json!({"kind":"integers","value":[]}),
            ),
            array_row(
                "indices",
                "",
                "integer",
                "500",
                json!({"kind":"integers","value":[]}),
            ),
        ];
        let expect = [
            (3usize, vec!["v0", "v1", "v2"], 2u64),
            (4, vec!["v0", "v1", "v2", "v3"], 2),
            (1, vec!["value"], 9),
            (1, vec!["value"], 500),
        ];
        for (row, (width, columns, groups)) in rows.iter().zip(expect) {
            let layout = table(row).expect("an array row makes a table");
            assert_eq!(layout.width, width, "{} x {}", row.name, row.implementation);
            assert_eq!(
                layout.columns, columns,
                "{} x {}",
                row.name, row.implementation
            );
            assert_eq!(
                layout.groups(),
                groups,
                "{} x {}",
                row.name,
                row.implementation
            );
        }
    }

    /// A kept page index stays inside the table's page range: the array
    /// can shrink while its row is open.
    #[test]
    fn the_kept_page_stays_inside_the_table_range() {
        let long = array_row(
            "radii",
            "",
            "float",
            "500",
            json!({"kind":"floats","value":[]}),
        );
        let layout = table(&long).unwrap();
        assert_eq!(layout.pages(), 5);
        assert_eq!(layout.clamped_page(4), 4, "the last page keeps its index");
        assert_eq!(
            layout.clamped_page(500),
            4,
            "a kept page past the end reads the last page"
        );
        let short = array_row(
            "radii",
            "",
            "float",
            "5",
            json!({"kind":"floats","value":[]}),
        );
        let layout = table(&short).unwrap();
        assert_eq!(layout.pages(), 1);
        assert_eq!(
            layout.clamped_page(4),
            0,
            "a single-page table reads the first page"
        );
    }

    /// Single values, resource lists and undeclared rows keep their
    /// current fields — no values to page through.
    #[test]
    fn rows_without_arrays_do_not_get_tables() {
        let empty = array_row(
            "radius",
            "",
            "float",
            "1",
            json!({"kind":"floats","value":[]}),
        );
        assert_eq!(table(&empty), None, "a single value keeps its field");
        let materials = array_row(
            "materials",
            "",
            "string",
            "2",
            json!({"kind":"resources","resource_kind":"material","ids":["a","b"]}),
        );
        assert_eq!(&materials.parameter["values"]["kind"], "resources");
        assert_eq!(table(&materials), None, "resource lists stay links");
        let undeclared = Row {
            declared: false,
            field: Field::Unset,
            hint: "not set".into(),
            parameter: Value::Null,
            ..empty
        };
        assert_eq!(table(&undeclared), None, "an undeclared row has no values");
    }

    /// The closed row names the type and the group count.
    #[test]
    fn the_summary_counts_groups() {
        let asset = array_row(
            "P",
            "",
            "point3",
            "5043",
            json!({"kind":"array","asset":"a1","encoding":"f64_le","count":"5043"}),
        );
        assert_eq!(table(&asset).unwrap().summary(), "point3 · 1681 groups");
        let values = array_row(
            "radii",
            "",
            "float",
            "5",
            json!({"kind":"floats","value":[]}),
        );
        assert_eq!(table(&values).unwrap().summary(), "float · 5 groups");
    }

    /// Windows page `PAGE_GROUPS` groups of `width` values and clip the
    /// tail to what is left.
    #[test]
    fn windows_clip_the_last_page() {
        let mesh = array_row(
            "P",
            "",
            "point3",
            "363",
            json!({"kind":"floats","value":[]}),
        );
        let layout = table(&mesh).unwrap();
        assert_eq!(layout.pages(), 2);
        assert_eq!(layout.window(0), (0, 300));
        assert_eq!(layout.window(1), (300, 63));
        let radii = array_row(
            "radii",
            "",
            "float",
            "500",
            json!({"kind":"floats","value":[]}),
        );
        let layout = table(&radii).unwrap();
        assert_eq!(layout.pages(), 5);
        assert_eq!(layout.window(4), (400, 100));
    }

    /// Cells read the preview's value forms.
    #[test]
    fn cells_read_the_value_forms() {
        assert_eq!(cell_text(&finite(1.5)), "1.5");
        assert_eq!(cell_text(&json!({"kind":"positive_infinity"})), "inf");
        assert_eq!(cell_text(&json!({"kind":"negative_infinity"})), "-inf");
        assert_eq!(cell_text(&json!({"kind":"nan"})), "nan");
        assert_eq!(cell_text(&json!("7")), "7");
        assert_eq!(cell_text(&json!(true)), "true");
        assert_eq!(cell_text(&json!("wood.png")), "wood.png");
        assert_eq!(cell_text(&json!("a1")), "a1");
        assert_eq!(cell_text(&Value::Null), "—");
    }

    /// Pages are asked once per revision, keep their answers, and reset
    /// when the project moves.
    #[test]
    fn pages_ask_once_per_revision_and_reset_on_change() {
        let mut pages = ValuesPages::default();
        pages.reset(json!({"revision":"a"}));
        assert!(pages.current(&json!({"revision":"a"})));
        assert!(
            !pages.current(&json!({"revision":"b"})),
            "another project is not current"
        );
        assert!(!pages.asked(0));
        pages.mark_asked(0);
        pages.mark_asked(0);
        assert!(pages.asked(0), "asking twice stays one pending entry");
        assert_eq!(pages.page(0), None, "pending has no payload yet");
        pages.answer(0, Ok(json!({"total":"300"})));
        assert_eq!(pages.page(0), Some(&json!({"total":"300"})));
        assert!(!pages.asked(1));
        pages.answer(1, Err("value offset exceeds count".into()));
        assert_eq!(
            pages.page(1),
            Some(&json!({"error":"value offset exceeds count"}))
        );
        pages.reset(json!({"revision":"b"}));
        assert!(!pages.asked(0) && !pages.asked(1), "a new project re-asks");
    }
}
