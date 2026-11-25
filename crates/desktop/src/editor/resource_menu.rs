//! Contents ▸ New menu rows for creating resources.
//! Pure so the placement and shapes can be tested without a window: the rows
//! come from the backend's static summary of r4's parameter table.
use serde_json::{Value, json};

/// The creatable implementations of one kind's summary
/// (`{"implementations": [...]}`); an empty summary lists nothing.
pub fn implementations(summary: &Value) -> Vec<String> {
    summary["implementations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// The apply edit for one creation: the label is the
/// implementation (generic for object definitions), the folder is the
/// selection's when the caller passes one, textures carry their value type.
pub fn edit(
    kind: &str,
    implementation: &str,
    texture_value_type: Option<&str>,
    folder: Option<&str>,
) -> Value {
    let label = default_label(kind, implementation);
    json!({
        "kind": "create_resource",
        "resource_kind": kind,
        "implementation": implementation,
        "label": label,
        "folder": folder,
        "texture_value_type": texture_value_type,
    })
}

/// The kind's default folder: the same rules as `project::layout`'s
/// kinds, repeated here because desktop does not depend on that crate.
pub fn default_folder(kind: &str) -> &'static str {
    match kind {
        "material" => "Materials",
        "texture" => "Textures",
        "medium" => "Media",
        "spectrum_data" => "Spectra",
        _ => "Objects",
    }
}

/// The label a new item starts with before the immediate rename.
pub fn default_label(kind: &str, implementation: &str) -> String {
    if kind == "object_definition" {
        "Object".into()
    } else if kind == "spectrum_data" {
        "Spectrum".into()
    } else {
        implementation.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(implementations: &[&str]) -> Value {
        json!({"implementations": implementations})
    }

    #[test]
    fn implementations_come_from_the_summary_and_survive_a_failed_fetch() {
        assert_eq!(
            implementations(&summary(&["diffuse", "conductor"])),
            vec!["diffuse", "conductor"]
        );
        // A failed fetch lands as null: no rows, not a panic.
        assert!(implementations(&Value::Null).is_empty());
        assert!(implementations(&json!({})).is_empty());
    }

    #[test]
    fn edits_shape_creations_and_default_their_labels() {
        let material = edit("material", "diffuse", None, Some("Materials"));
        assert_eq!(material["resource_kind"], "material");
        assert_eq!(material["implementation"], "diffuse");
        assert_eq!(material["label"], "diffuse");
        assert_eq!(material["folder"], "Materials");
        // Textures carry their value type; a missing folder lets the server
        // pick the kind's default.
        let texture = edit("texture", "checkerboard", Some("float"), None);
        assert_eq!(texture["texture_value_type"], "float");
        assert!(texture["folder"].is_null());
        // Object definitions start from a generic label.
        let definition = edit("object_definition", "", None, None);
        assert_eq!(definition["label"], "Object");
        assert_eq!(definition["implementation"], "");
    }

    #[test]
    fn kinds_keep_their_default_folders() {
        assert_eq!(default_folder("material"), "Materials");
        assert_eq!(default_folder("texture"), "Textures");
        assert_eq!(default_folder("medium"), "Media");
        assert_eq!(default_folder("object_definition"), "Objects");
        assert_eq!(default_folder("spectrum_data"), "Spectra");
    }

    #[test]
    fn spectrum_creations_start_as_a_flat_unit_curve_in_spectra() {
        let spectrum = edit("spectrum_data", "piecewise", None, None);
        assert_eq!(spectrum["resource_kind"], "spectrum_data");
        assert_eq!(spectrum["implementation"], "piecewise");
        assert_eq!(spectrum["label"], "Spectrum");
        assert!(spectrum["folder"].is_null());
    }
}
