//! The Inspector's Implementation row. Pure so the
//! shapes can be tested without a window: which implementations a component
//! box offers, which target the change edits, and what the confirmation
//! dialog says before `project.plan_implementation`'s edits are applied.
use serde_json::Value;

/// The candidates for one component box, from the startup schema summary
/// (`Update::ResourceSchemas`): the parameter table's rows, else the export
/// schema's list (Camera and the configuration directives). Textures offer
/// only their own value type's rows. A cancelling area light (empty
/// implementation) has nothing to change and offers nothing.
pub fn choices(component: &Value, schema: &Value) -> Vec<String> {
    if component["implementation"]
        .as_str()
        .is_none_or(str::is_empty)
    {
        return Vec::new();
    }
    let key = match component["kind"].as_str().unwrap_or_default() {
        "texture" => match component["texture_value_type"].as_str() {
            Some(value_type) => format!("texture_{value_type}"),
            None => return Vec::new(),
        },
        kind @ ("shape" | "light" | "area_light" | "camera" | "film" | "sampler" | "integrator"
        | "pixel_filter" | "accelerator" | "material" | "medium") => kind.to_owned(),
        _ => return Vec::new(),
    };
    schema[key]["implementations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect()
}

/// What the dialog says before a plan's edits go out, or `None` when the
/// change keeps every parameter silently (it is sent without asking).
pub fn confirmation(plan: &Value) -> Option<String> {
    let names = |key: &str| {
        plan[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (removed, unknown) = (names("breaks_reference"), names("kept_unknown"));
    let implementation = plan["implementation"].as_str().unwrap_or_default();
    let mut lines = Vec::new();
    if !removed.is_empty() {
        lines.push(format!(
            "Removed first ({implementation} cannot export them): {removed}"
        ));
    }
    if !unknown.is_empty() {
        lines.push(format!(
            "Kept, but not in {implementation}'s parameter table: {unknown}"
        ));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "material": {"implementations": ["diffuse", "conductor"]},
            "texture_float": {"implementations": ["constant"]},
            "texture_spectrum": {"implementations": ["constant", "marble"]},
            "integrator": {"implementations": ["path", "volpath"]},
            "area_light": {"implementations": ["diffuse"]},
        })
    }

    #[test]
    fn choices_follow_the_component_kind() {
        let schema = schema();
        let names = |component: Value| choices(&component, &schema);
        assert_eq!(
            names(json!({"kind":"material","implementation":"diffuse"})),
            ["diffuse", "conductor"]
        );
        assert_eq!(
            names(json!({"kind":"integrator","implementation":"path"})),
            ["path", "volpath"]
        );
        // Textures stay within their value type.
        assert_eq!(
            names(json!({"kind":"texture","implementation":"constant",
                         "texture_value_type":"float"})),
            ["constant"]
        );
        assert_eq!(
            names(json!({"kind":"texture","implementation":"constant",
                         "texture_value_type":"spectrum"})),
            ["constant", "marble"]
        );
        // The cancelling area light and kinds without an implementation
        // offer nothing.
        assert!(names(json!({"kind":"area_light","implementation":""})).is_empty());
        assert!(names(json!({"kind":"transform"})).is_empty());
        assert!(names(json!({"kind":"object_definition","implementation":""})).is_empty());
    }

    #[test]
    fn the_dialog_names_removed_and_unknown_parameters() {
        let silent = json!({"implementation":"disk","kept_unknown":[],"breaks_reference":[]});
        assert_eq!(confirmation(&silent), None);
        let plan = json!({"implementation":"diffuse","kept_unknown":["amount"],
                          "breaks_reference":["materials"]});
        assert_eq!(
            confirmation(&plan).unwrap(),
            "Removed first (diffuse cannot export them): materials\n\
             Kept, but not in diffuse's parameter table: amount"
        );
        let plan = json!({"implementation":"volpath","kept_unknown":["maxdepth","regularize"],
                          "breaks_reference":[]});
        assert_eq!(
            confirmation(&plan).unwrap(),
            "Kept, but not in volpath's parameter table: maxdepth, regularize"
        );
    }
}
