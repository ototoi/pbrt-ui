//! Accumulate `project.search_contents` pages without mixing query revisions.
use serde_json::Value;
use std::collections::HashSet;

/// How many entries one Contents search page asks the core for. The panel
///The typing and submit paths, and the session's revision re-run, preserve the query
/// survives a revision and is re-issued) share it.
pub(super) const SEARCH_PAGE_LIMIT: usize = 64;

fn key(entry: &Value) -> String {
    match (entry["kind"].as_str(), entry["id"].as_str()) {
        (Some(kind), Some(id)) if entry["builtin"] == true => format!("builtin:{kind}:{id}"),
        (Some(kind), Some(id)) => format!("{kind}:{id}"),
        _ => format!(
            "{}:{}",
            entry["kind"].as_str().unwrap_or_default(),
            entry["location"].as_str().unwrap_or_default()
        ),
    }
}

pub fn request_is_current(
    project: &Value,
    query: &str,
    generation: u64,
    current_project: &Value,
    current_query: &str,
    current_generation: u64,
) -> bool {
    generation == current_generation && project == current_project && query == current_query
}

pub fn response_is_current(
    project: &Value,
    query: &str,
    generation: u64,
    current_project: &Value,
    current_query: &str,
    current_generation: u64,
    page: &Value,
) -> bool {
    request_is_current(
        project,
        query,
        generation,
        current_project,
        current_query,
        current_generation,
    ) && page["project"] == *project
        && page["query"] == query
}

pub fn apply_page(current: &mut Value, offset: usize, page: &Value) -> bool {
    if page.is_null() {
        if offset == 0 {
            *current = Value::Null;
        }
        return false;
    }
    merge_page(current, offset, page).is_some()
}

pub fn merge_page(current: &mut Value, offset: usize, page: &Value) -> Option<()> {
    let incoming = page.as_object()?;
    if offset == 0 || !current.is_object() {
        *current = page.clone();
        return Some(());
    }
    let current = current.as_object_mut()?;
    let existing = current.get_mut("entries")?.as_array_mut()?;
    let mut seen: HashSet<_> = existing.iter().map(key).collect();
    for entry in incoming.get("entries")?.as_array()? {
        if seen.insert(key(entry)) {
            existing.push(entry.clone());
        }
    }
    for field in ["project", "query", "total", "next_offset"] {
        current.insert(
            field.into(),
            incoming.get(field).cloned().unwrap_or(Value::Null),
        );
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::merge_page;
    use serde_json::{Value, json};

    #[test]
    fn search_pages_replace_then_append_without_duplicate_entries() {
        let project = json!({"revision":"1"});
        let mut result = json!(null);
        merge_page(
            &mut result,
            0,
            &json!({"project":project,"query":"wood","entries":[
                {"kind":"resource","id":"a","location":"A"}
            ],"total":3,"next_offset":1}),
        )
        .unwrap();
        merge_page(
            &mut result,
            1,
            &json!({"project":project,"query":"wood","entries":[
                {"kind":"resource","id":"a","location":"A"},
                {"kind":"folder","location":"Wood"}
            ],"total":2,"next_offset":null}),
        )
        .unwrap();
        assert_eq!(result["entries"].as_array().unwrap().len(), 2);
        assert_eq!(result["total"], 2);
        assert!(result["next_offset"].is_null());
        merge_page(
            &mut result,
            0,
            &json!({"query":"oak","entries":[],"total":0}),
        )
        .unwrap();
        assert_eq!(result["query"], "oak");
        assert_eq!(result["entries"], Value::Array(vec![]));
    }

    #[test]
    fn stale_project_query_and_generation_responses_are_rejected() {
        let project = json!({"revision":"1"});
        let other_project = json!({"revision":"2"});
        let page = json!({"project":project,"query":"wood","entries":[]});
        assert!(super::response_is_current(
            &project, "wood", 3, &project, "wood", 3, &page
        ));
        assert!(!super::response_is_current(
            &other_project,
            "wood",
            3,
            &project,
            "wood",
            3,
            &page
        ));
        assert!(!super::response_is_current(
            &project, "oak", 3, &project, "wood", 3, &page
        ));
        assert!(!super::response_is_current(
            &project, "wood", 2, &project, "wood", 3, &page
        ));
        assert!(super::request_is_current(
            &project, "wood", 3, &project, "wood", 3
        ));
    }

    #[test]
    fn failed_continuation_keeps_existing_rows_for_retry() {
        let mut result = json!({
            "project":{"revision":"1"},"query":"wood",
            "entries":[{"kind":"resource","id":"r","location":"Wood/oak"}],
            "total":2,"next_offset":1
        });
        assert!(!super::apply_page(&mut result, 1, &Value::Null));
        assert_eq!(result["entries"].as_array().unwrap().len(), 1);
        assert_eq!(result["next_offset"], 1);
        assert!(super::apply_page(
            &mut result,
            1,
            &json!({
                "project":{"revision":"1"},"query":"wood",
                "entries":[{"kind":"resource","id":"s","location":"Wood/pine"}],
                "total":2,"next_offset":null
            })
        ));
        assert_eq!(result["entries"].as_array().unwrap().len(), 2);
    }
}
