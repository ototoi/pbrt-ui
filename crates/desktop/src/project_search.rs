//! Accumulate project-node search pages without re-scanning the tree.
use serde_json::Value;
use std::collections::HashSet;

/// Merge one `project.search_nodes` page into the accumulated result.
///
/// `offset == 0` replaces the previous
/// result; later offsets append. Appends are de-duplicated by `node_id` so a
/// retried page cannot render the same hit twice. `total` and `next_offset`
/// always come from the newest page.
pub fn merge_page(current: &mut Value, offset: usize, page: &Value) -> Option<()> {
    let incoming = page.as_object()?;
    if offset == 0 || !current.is_object() {
        *current = page.clone();
        return Some(());
    }
    let current = current.as_object_mut()?;
    let existing = current.get_mut("nodes")?.as_array_mut()?;
    let mut seen: HashSet<String> = existing
        .iter()
        .filter_map(|node| node["node_id"].as_str().map(str::to_owned))
        .collect();
    for node in incoming.get("nodes")?.as_array()? {
        let Some(id) = node["node_id"].as_str() else {
            continue;
        };
        if seen.insert(id.to_owned()) {
            existing.push(node.clone());
        }
    }
    current.insert(
        "total".into(),
        incoming.get("total").cloned().unwrap_or(Value::Null),
    );
    current.insert(
        "next_offset".into(),
        incoming.get("next_offset").cloned().unwrap_or(Value::Null),
    );
    Some(())
}

#[cfg(test)]
mod tests {
    use super::merge_page;
    use serde_json::{Value, json};

    fn page(ids: &[&str], total: u64, next_offset: Option<u64>) -> Value {
        let nodes = ids
            .iter()
            .map(|id| json!({"node_id":id,"label":id,"kind":"entity","path":[]}))
            .collect::<Vec<_>>();
        json!({"nodes":nodes,"total":total,"next_offset":next_offset})
    }

    fn ids(value: &Value) -> Vec<&str> {
        value["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["node_id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn offset_zero_replaces_and_later_offsets_append() {
        let mut current = page(&["a"], 3, Some(1));
        merge_page(&mut current, 0, &page(&["a", "b"], 3, Some(2))).unwrap();
        assert_eq!(ids(&current), vec!["a", "b"]);
        assert_eq!(current["next_offset"], 2);
        merge_page(&mut current, 2, &page(&["c"], 3, None)).unwrap();
        assert_eq!(ids(&current), vec!["a", "b", "c"]);
        assert!(current["next_offset"].is_null());
        assert_eq!(current["total"], 3);
    }

    #[test]
    fn appended_pages_are_deduplicated_by_node_id() {
        let mut current = page(&["a", "b"], 4, Some(2));
        merge_page(&mut current, 2, &page(&["b", "c", "c"], 4, Some(3))).unwrap();
        assert_eq!(ids(&current), vec!["a", "b", "c"]);
        // A retried page is harmless.
        merge_page(&mut current, 2, &page(&["b", "c"], 4, Some(3))).unwrap();
        assert_eq!(ids(&current), vec!["a", "b", "c"]);
    }

    #[test]
    fn empty_or_invalid_accumulators_and_pages_are_handled() {
        let mut current = json!(null);
        merge_page(&mut current, 2, &page(&["a"], 1, None)).unwrap();
        assert_eq!(ids(&current), vec!["a"]);

        let mut current = page(&["a"], 1, None);
        let before = current.clone();
        assert!(merge_page(&mut current, 1, &json!(null)).is_none());
        assert_eq!(current, before);
        assert!(merge_page(&mut current, 1, &json!("not a page")).is_none());
        assert_eq!(current, before);
    }
}
