//! Inspector components: titles and icons, the folded component kinds
//! It builds component rows, links, and selector choices,
//! and the swatch painters the Inspector and the node editor share.
//! Pure so it can be tested without a window.
use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyView, App, Bounds, IntoElement, PathBuilder, Styled, Window, canvas, point, px, rgb,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

/// Display name and icon per `components[].kind`.
pub fn title(kind: &str) -> (&'static str, IconName) {
    match kind {
        "scene" => ("Scene", IconName::Axis3d),
        "transform" => ("Transform", IconName::Move3d),
        "shape" => ("Shape", IconName::Triangle),
        "light" => ("Light", IconName::Lightbulb),
        "area_light" => ("Area Light", IconName::LampCeiling),
        "reverse_orientation" => ("Reverse Orientation", IconName::RefreshCw),
        "alpha_mask" => ("Alpha Mask", IconName::Scan),
        "material" => ("Material", IconName::Palette),
        "medium" => ("Medium", IconName::Droplets),
        "object_instance" => ("Object Instance", IconName::Copy),
        "camera" => ("Camera", IconName::Camera),
        "render_settings" => ("Render Settings", IconName::Settings),
        "film" => ("Film", IconName::Film),
        "sampler" => ("Sampler", IconName::Dices),
        "integrator" => ("Integrator", IconName::Sigma),
        "pixel_filter" => ("Pixel Filter", IconName::Funnel),
        "accelerator" => ("Accelerator", IconName::Network),
        "color_space" => ("Color Space", IconName::Palette),
        "options" => ("Options", IconName::Settings),
        "texture" => ("Texture", IconName::Image),
        "mapping" => ("Mapping", IconName::Grid2x2),
        "object_definition" => ("Object Definition", IconName::Boxes),
        _ => ("Component", IconName::Box),
    }
}

/// Folded component kinds, shared by every node and kept per user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folded(BTreeSet<String>);

impl Folded {
    /// A missing or malformed file folds nothing.
    pub fn parse(bytes: &[u8]) -> Self {
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
            return Self::default();
        };
        if value["version"] != 1 {
            return Self::default();
        }
        Self(
            value["collapsed"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|kind| {
                    kind.as_str().map(|kind| {
                        if kind == "medium_interface" {
                            "medium".to_owned()
                        } else {
                            kind.to_owned()
                        }
                    })
                })
                .collect(),
        )
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .map(|bytes| Self::parse(&bytes))
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> Value {
        json!({"version": 1, "collapsed": self.0.iter().collect::<Vec<_>>()})
    }

    /// Write through a temporary file and rename.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
        std::fs::create_dir_all(parent)?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(&self.to_json()).map_err(std::io::Error::other)?;
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)
    }

    pub fn is_folded(&self, kind: &str) -> bool {
        self.0.contains(kind)
    }

    pub fn toggle(&mut self, kind: &str) {
        if !self.0.remove(kind) {
            self.0.insert(kind.to_owned());
        }
    }

    pub fn unfold(&mut self, kind: &str) {
        self.0.remove(kind);
    }
}

/// The name a resource or asset id shows as, from the revision's items.
pub fn item_name(items: &Value, id: &str) -> String {
    items[id]["name"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}…", id.get(..8).unwrap_or(id)))
}

/// (items 9 and 10): a hover tooltip builder over GPUI's own
/// tooltip draw, making any stateful div a tooltip trigger. gpui-kit's
/// managed tooltip lives on its own components (Button and kin), so a
/// plain truncated div — the value label beside a spectrum row's box,
/// the Metal colour's source — spells its full text through this.
pub fn text_tooltip(text: String) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
}

/// The empty look of a swatch with no resolved colour
/// in it — outlined and slashed corner to corner in the muted grey the
/// undefined rows share. The plain filled `muted` dark read as "0"; a
/// hollow box reads as nothing. One painter for the Inspector's colour
/// box and the node editor's swatches, drawing in whatever bounds it
/// gets — size it through its parent (`size_full` on the canvas).
pub fn unset_swatch() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |bounds: Bounds<_>, _, window: &mut Window, _| {
            let paint = rgb(0x676d78);
            let at = |x: f32, y: f32| {
                point(
                    bounds.origin.x + px(x) + px(0.5),
                    bounds.origin.y + px(y) + px(0.5),
                )
            };
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let inset = 1.;
            let mut border = PathBuilder::stroke(px(1.));
            border.move_to(at(inset, inset));
            border.line_to(at(width - inset, inset));
            border.line_to(at(width - inset, height - inset));
            border.line_to(at(inset, height - inset));
            border.close();
            let mut slash = PathBuilder::stroke(px(1.));
            slash.move_to(at(inset, inset));
            slash.line_to(at(width - inset, height - inset));
            if let Ok(path) = border.build() {
                window.paint_path(path, paint);
            }
            if let Ok(path) = slash.build() {
                window.paint_path(path, paint);
            }
        },
    )
    .size_full()
}

/// The text after a component's title: its implementation, or what it
/// points at.
pub fn subtitle(component: &Value, items: &Value) -> String {
    let reference = |id: &Value| id.as_str().map(|id| item_name(items, id));
    match component["kind"].as_str().unwrap_or("") {
        // A material resource box has no binding; it names its implementation.
        "material" if component["binding"].is_null() => component["implementation"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        "material" => match component["binding"]["kind"].as_str() {
            Some("resource") => reference(&component["binding"]["value"])
                .map(|name| format!("→ {name}"))
                .unwrap_or_default(),
            Some("none") => "Interface".into(),
            _ => "default".into(),
        },
        "medium" if component.get("inside").is_some() || component.get("outside").is_some() => {
            let side = |key: &str| reference(&component[key]).unwrap_or_else(|| "—".into());
            format!("{} / {}", side("inside"), side("outside"))
        }
        "medium" if component.get("binding").is_some() => {
            reference(&component["binding"]).unwrap_or_else(|| "None".into())
        }
        "medium" => component["implementation"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        "reverse_orientation" if component["enabled"] == true => "on".into(),
        "reverse_orientation" => "off".into(),
        "object_instance" => reference(&component["definition"])
            .map(|name| format!("→ {name}"))
            .unwrap_or_default(),
        "color_space" => component["value"].as_str().unwrap_or_default().to_owned(),
        // The Scene box names the scene's coordinate system.
        "scene" => {
            let system = &component["coordinate_system"];
            format!(
                "{} up · {}-handed",
                up_value(system).as_str().unwrap_or_default(),
                handedness_value(system).as_str().unwrap_or_default()
            )
        }
        _ => component["implementation"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    }
}

/// Selector choices: (label, value) with the binding the wire expects.
/// Materials offer `Default` and `Interface`; media offer `None`.
pub fn material_choices(items: &Value) -> Vec<(String, Value)> {
    std::iter::once(("Default".to_owned(), json!({"kind":"default"})))
        .chain(std::iter::once((
            "Interface".to_owned(),
            json!({"kind":"none"}),
        )))
        .chain(
            resources(items, "material")
                .into_iter()
                .map(|(id, name)| (name, json!({"kind":"resource","value":id}))),
        )
        .collect()
}

/// Create a material with a caller-chosen id and bind it in the same apply.
pub fn new_material_edits(node_id: &str, implementation: &str, id: &str) -> Vec<Value> {
    let mut create =
        crate::editor::resource_menu::edit("material", implementation, None, Some("Materials"));
    create["id"] = json!(id);
    vec![
        create,
        material_edit(node_id, &json!({"kind":"resource","value":id})),
    ]
}

pub fn medium_choices(items: &Value) -> Vec<(String, Value)> {
    std::iter::once(("None".to_owned(), Value::Null))
        .chain(
            resources(items, "medium")
                .into_iter()
                .map(|(id, name)| (name, json!(id))),
        )
        .collect()
}

/// (id, name) of one resource kind, sorted by name.
fn resources(items: &Value, kind: &str) -> Vec<(String, String)> {
    let mut list = items
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, entry)| entry["resource_kind"] == kind)
        .map(|(id, entry)| {
            (
                id.clone(),
                entry["name"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    list.sort_by(|a, b| {
        a.1.to_lowercase()
            .cmp(&b.1.to_lowercase())
            .then(a.0.cmp(&b.0))
    });
    list
}

/// Object definitions an instance may point at.
pub fn definition_choices(items: &Value) -> Vec<(String, Value)> {
    resources(items, "object_definition")
        .into_iter()
        .map(|(id, name)| (name, json!(id)))
        .collect()
}

/// `set_instance_definition` for one instance.
pub fn definition_edit(node_id: &str, definition: &Value) -> Value {
    json!({"kind":"set_instance_definition","node_id":node_id,"definition":definition})
}

/// The Scene box's up axis choices: the wire's signed words, in
/// X→Y→Z order.
pub fn up_choices() -> Vec<(String, Value)> {
    ["+X", "-X", "+Y", "-Y", "+Z", "-Z"]
        .into_iter()
        .map(|word| (word.to_owned(), json!(word)))
        .collect()
}

/// The Scene box's handedness choices: display labels over the wire's
/// words.
pub fn handedness_choices() -> Vec<(String, Value)> {
    [("Left", "left"), ("Right", "right")]
        .into_iter()
        .map(|(label, word)| (label.to_owned(), json!(word)))
        .collect()
}

/// The scene's current up axis in its wire word. A missing or
/// unrecognized report falls back to the new-scene default, +Y.
pub fn current_up(coordinate_system: &Value) -> String {
    up_value(coordinate_system)
        .as_str()
        .unwrap_or("+Y")
        .to_owned()
}

/// The scene's handedness as a display label. A missing or unrecognized
/// report falls back to the new-scene default, Left.
pub fn current_handedness(coordinate_system: &Value) -> String {
    match handedness_value(coordinate_system).as_str() {
        Some("right") => "Right",
        _ => "Left",
    }
    .to_owned()
}

/// The scene's current up axis as the wire value, with the +Y default.
pub fn up_value(coordinate_system: &Value) -> Value {
    match coordinate_system["up"].as_str() {
        Some(word @ ("+X" | "-X" | "+Y" | "-Y" | "+Z" | "-Z")) => json!(word),
        _ => json!("+Y"),
    }
}

/// The scene's current handedness as the wire value, with the left
/// default.
pub fn handedness_value(coordinate_system: &Value) -> Value {
    match coordinate_system["handedness"].as_str() {
        Some("right") => json!("right"),
        _ => json!("left"),
    }
}

/// One `set_scene_coordinate_system` edit for the Scene box: the chosen
/// field with the scene's other field kept. Metadata only — the
/// core moves no node.
pub fn coordinate_edit(scene: &str, up: &Value, handedness: &Value) -> Value {
    json!({"kind":"set_scene_coordinate_system", "scene":scene,
           "up":up, "handedness":handedness})
}

/// A byte count (the wire sends it as a decimal string) for display.
pub fn size_text(bytes: &str) -> String {
    let Ok(bytes) = bytes.parse::<u64>() else {
        return bytes.to_owned();
    };
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KiB", bytes as f64 / 1024.),
        _ => format!("{:.1} MiB", bytes as f64 / 1_048_576.),
    }
}

/// `set_local_binding` for a new material on one object.
pub fn material_edit(node_id: &str, binding: &Value) -> Value {
    json!({"kind":"set_local_binding","node_id":node_id,"material":binding,"medium":null})
}

/// `set_local_binding` for one side of the medium interface, keeping the other.
pub fn medium_edit(node_id: &str, component: &Value, inside: bool, medium: &Value) -> Value {
    let (inside_id, outside_id) = if inside {
        (medium.clone(), component["outside"].clone())
    } else {
        (component["inside"].clone(), medium.clone())
    };
    json!({
        "kind":"set_local_binding","node_id":node_id,"material":null,
        "medium":{"inside":inside_id,"outside":outside_id},
    })
}

/// Set a camera's single ray-medium binding while preserving its stored
/// inside value for PBRT round-tripping.
pub fn camera_medium_edit(node_id: &str, component: &Value, medium: &Value) -> Value {
    json!({
        "kind":"set_local_binding","node_id":node_id,"material":null,
        "medium":{"inside":component["preserved_inside"],"outside":medium},
    })
}

/// Rename a node or resource from the Inspector header.
pub fn rename_edit(details: &Value, label: &str) -> Option<Value> {
    let label = label.trim();
    if label.is_empty() {
        return None;
    }
    if let Some(id) = details["resource"]["resource_id"].as_str() {
        if details["resource"]["label"].as_str() == Some(label) {
            return None;
        }
        return Some(json!({"kind":"rename_resource","resource_id":id,"label":label}));
    }
    let node = &details["node"];
    if node["data"]["kind"] == "scene" || node["label"].as_str() == Some(label) {
        return None;
    }
    Some(json!({"kind":"rename_node","node_id":node["node_id"].as_str()?,"label":label}))
}

/// Parameter values as text: `1 0.5 2`, `inf`, `"name"`, `true`.
pub fn values_text(values: &Value) -> String {
    let one = |value: &Value| match value {
        Value::Object(float) => match float.get("kind").and_then(Value::as_str) {
            Some("finite") => float.get("value").map(Value::to_string).unwrap_or_default(),
            Some("positive_infinity") => "inf".into(),
            Some("negative_infinity") => "-inf".into(),
            Some("nan") => "nan".into(),
            _ => value.to_string(),
        },
        Value::String(text) => match values["kind"].as_str() {
            Some("integers") => text.clone(),
            _ => format!("\"{text}\""),
        },
        other => other.to_string(),
    };
    match values["kind"].as_str() {
        Some("resources") => values["ids"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|id| id.as_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>()
            .join(" "),
        Some("array") => format!(
            "{} values in a data asset",
            values["count"].as_str().unwrap_or("?")
        ),
        _ => values["value"]
            .as_array()
            .into_iter()
            .flatten()
            .map(one)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// One row of a 4×4 matrix for display.
/// Convert a TRS value from the wire into Position, Rotation, and Scale rows.
pub fn trs_rows(trs: &Value) -> Vec<(&'static str, String)> {
    [
        ("Position", "translation"),
        ("Rotation", "rotation_euler_degrees"),
        ("Scale", "scale"),
    ]
    .into_iter()
    .map(|(label, key)| {
        let text = ["X", "Y", "Z"]
            .iter()
            .zip(trs[key].as_array().into_iter().flatten())
            .map(|(axis, v)| {
                // Round first so -0.0001 shows as 0.000; `+ 0.` drops -0.
                let v = (v.as_f64().unwrap_or(f64::NAN) * 1000.).round() / 1000. + 0.;
                format!("{axis} {v:.3}")
            })
            .collect::<Vec<_>>()
            .join("  ");
        (label, text)
    })
    .collect()
}

/// Why a transform has no local matrix or no TRS, as a phrase for
/// "The local transform …" / "TRS unavailable: the matrix …".
pub fn transform_reason(reason: &Value) -> Option<&'static str> {
    Some(match reason.as_str()? {
        "animated" => "changes over the shutter interval",
        "coordinate_system" => "uses a named coordinate system",
        "singular" => "is singular",
        "shear" => "has shear",
        "projective" => "is projective",
        "non_finite" => "is not finite",
        _ => "cannot be expressed",
    })
}

pub fn matrix_rows(values: &Value) -> Vec<String> {
    let numbers = values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect::<Vec<_>>();
    numbers
        .chunks(4)
        .map(|row| {
            row.iter()
                .map(|value| format!("{value:.3}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trs_rows_show_three_decimals_and_no_negative_zero() {
        let trs = serde_json::json!({
            "translation":[0.0, 2.8, -0.0],
            "rotation_euler_degrees":[-0.00001, 45.0, -180.0],
            "rotation_order":"zxy",
            "scale":[-1.0, 1.0, 2.5],
        });
        assert_eq!(
            trs_rows(&trs),
            [
                ("Position", "X 0.000  Y 2.800  Z 0.000".to_owned()),
                ("Rotation", "X 0.000  Y 45.000  Z -180.000".to_owned()),
                ("Scale", "X -1.000  Y 1.000  Z 2.500".to_owned()),
            ]
        );
        assert_eq!(
            transform_reason(&serde_json::json!("shear")),
            Some("has shear")
        );
        assert_eq!(transform_reason(&Value::Null), None);
    }

    fn items() -> Value {
        json!({
            "m1": {"kind":"resource","resource_kind":"material","name":"wall"},
            "m2": {"kind":"resource","resource_kind":"material","name":"Brick"},
            "fog": {"kind":"resource","resource_kind":"medium","name":"fog"},
            "a": {"kind":"asset","name":"bunny.ply"}
        })
    }

    #[test]
    fn components_fold_by_kind_and_survive_the_file() {
        let mut folded = Folded::default();
        folded.toggle("medium");
        assert!(folded.is_folded("medium"));
        assert!(!folded.is_folded("transform"));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pbrt-ui/inspector.json");
        folded.save(&path).unwrap();
        assert_eq!(Folded::load(&path), folded);
        assert_eq!(
            folded.to_json(),
            json!({"version":1,"collapsed":["medium"]})
        );
        folded.toggle("medium");
        assert!(!folded.is_folded("medium"));
        assert!(
            Folded::parse(br#"{"version":1,"collapsed":["medium_interface"]}"#).is_folded("medium")
        );
        assert_eq!(Folded::parse(b"broken"), Folded::default());
    }

    #[test]
    fn components_subtitles_name_what_they_point_at() {
        let items = items();
        let material = json!({"kind":"material","binding":{"kind":"resource","value":"m1"}});
        assert_eq!(subtitle(&material, &items), "→ wall");
        let default = json!({"kind":"material","binding":{"kind":"default"}});
        assert_eq!(subtitle(&default, &items), "default");
        let interface = json!({"kind":"material","binding":{"kind":"none"}});
        assert_eq!(subtitle(&interface, &items), "Interface");
        let resource = json!({"kind":"material","implementation":"conductor"});
        assert_eq!(subtitle(&resource, &items), "conductor");
        let medium = json!({"kind":"medium","inside":"fog","outside":null});
        assert_eq!(subtitle(&medium, &items), "fog / —");
        let camera_medium = json!({"kind":"medium","binding":"fog","preserved_inside":null});
        assert_eq!(subtitle(&camera_medium, &items), "fog");
        assert_eq!(
            subtitle(&json!({"kind":"medium","binding":null}), &items),
            "None"
        );
        assert_eq!(
            subtitle(
                &json!({"kind":"medium","implementation":"homogeneous"}),
                &items
            ),
            "homogeneous"
        );
        let shape = json!({"kind":"shape","implementation":"trianglemesh"});
        assert_eq!(subtitle(&shape, &items), "trianglemesh");
        assert_eq!(item_name(&items, "0123456789"), "01234567…");
    }

    #[test]
    fn camera_medium_edit_preserves_the_ignored_inside_binding() {
        let component = json!({"binding":"old-outside","preserved_inside":"legacy-inside"});
        let edit = camera_medium_edit("camera", &component, &json!("new-medium"));
        assert_eq!(edit["medium"]["outside"], "new-medium");
        assert_eq!(edit["medium"]["inside"], "legacy-inside");
    }

    #[test]
    fn components_selectors_send_local_bindings() {
        let items = items();
        let materials = material_choices(&items);
        assert_eq!(
            materials
                .iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            ["Default", "Interface", "Brick", "wall"]
        );
        assert_eq!(
            material_edit("n", &materials[2].1),
            json!({"kind":"set_local_binding","node_id":"n","material":{"kind":"resource","value":"m2"},"medium":null})
        );
        assert_eq!(
            material_edit("n", &materials[1].1),
            json!({"kind":"set_local_binding","node_id":"n","material":{"kind":"none"},"medium":null})
        );
        let media = medium_choices(&items);
        assert_eq!(media.len(), 2);
        let component = json!({"inside":null,"outside":"fog"});
        assert_eq!(
            medium_edit("n", &component, true, &json!("fog")),
            json!({"kind":"set_local_binding","node_id":"n","material":null,"medium":{"inside":"fog","outside":"fog"}})
        );
    }

    #[test]
    fn new_material_edits_create_and_bind_with_the_same_id() {
        let edits = new_material_edits("node", "conductor", "material-id");
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0]["kind"], "create_resource");
        assert_eq!(edits[0]["resource_kind"], "material");
        assert_eq!(edits[0]["folder"], "Materials");
        assert_eq!(edits[0]["id"], "material-id");
        assert_eq!(
            edits[1],
            json!({"kind":"set_local_binding","node_id":"node","material":{"kind":"resource","value":"material-id"},"medium":null})
        );
    }

    #[test]
    fn components_instances_choose_object_definitions() {
        let items = json!({
            "o1": {"kind":"resource","resource_kind":"object_definition","name":"tree"},
            "m1": {"kind":"resource","resource_kind":"material","name":"wall"}
        });
        let choices = definition_choices(&items);
        assert_eq!(choices, vec![("tree".to_owned(), json!("o1"))]);
        assert_eq!(
            definition_edit("n", &choices[0].1),
            json!({"kind":"set_instance_definition","node_id":"n","definition":"o1"})
        );
        assert_eq!(size_text("512"), "512 B");
        assert_eq!(size_text("2048"), "2.0 KiB");
        assert_eq!(size_text("3145728"), "3.0 MiB");
    }

    #[test]
    fn components_rename_nodes_and_resources() {
        let node = json!({"node":{"node_id":"n","label":"Floor","data":{"kind":"entity"}}});
        assert_eq!(
            rename_edit(&node, " Ground "),
            Some(json!({"kind":"rename_node","node_id":"n","label":"Ground"}))
        );
        assert_eq!(rename_edit(&node, "Floor"), None);
        assert_eq!(rename_edit(&node, "  "), None);
        let scene = json!({"node":{"node_id":"s","label":"Main","data":{"kind":"scene"}}});
        assert_eq!(rename_edit(&scene, "Other"), None);
        let resource = json!({"item":{},"resource":{"resource_id":"r","label":"wall"}});
        assert_eq!(
            rename_edit(&resource, "plaster"),
            Some(json!({"kind":"rename_resource","resource_id":"r","label":"plaster"}))
        );
    }

    #[test]
    fn components_show_parameter_values_as_text() {
        let floats = json!({"kind":"floats","value":[{"kind":"finite","value":1.5},{"kind":"positive_infinity"}]});
        assert_eq!(values_text(&floats), "1.5 inf");
        assert_eq!(
            values_text(&json!({"kind":"integers","value":["3","4"]})),
            "3 4"
        );
        assert_eq!(
            values_text(&json!({"kind":"strings","value":["a"]})),
            "\"a\""
        );
        assert_eq!(
            values_text(&json!({"kind":"booleans","value":[true]})),
            "true"
        );
        assert_eq!(
            values_text(&json!({"kind":"array","count":"300"})),
            "300 values in a data asset"
        );
    }

    #[test]
    fn components_titles_cover_the_core_kinds() {
        for kind in [
            "transform",
            "shape",
            "light",
            "area_light",
            "material",
            "medium",
            "object_instance",
            "camera",
            "film",
            "sampler",
            "integrator",
            "pixel_filter",
            "accelerator",
            "color_space",
            "options",
        ] {
            assert_ne!(title(kind).0, "Component", "{kind}");
        }
        assert_eq!(title("medium").0, "Medium");
        assert_eq!(matrix_rows(&json!([1, 0, 0, 0, 0, 1, 0, 0])).len(), 2);
    }

    /// The Scene box offers the six up axes and both handednesses
    /// in the wire's exact words, defaults to +Y/left when the report is
    /// missing or unknown, and its edits keep the other field.
    #[test]
    fn components_scene_box_edits_the_coordinate_system() {
        assert_eq!(title("scene").0, "Scene");
        assert_eq!(
            up_choices()
                .iter()
                .map(|(label, value)| { (label.as_str(), value.as_str().unwrap_or_default()) })
                .collect::<Vec<_>>(),
            [
                ("+X", "+X"),
                ("-X", "-X"),
                ("+Y", "+Y"),
                ("-Y", "-Y"),
                ("+Z", "+Z"),
                ("-Z", "-Z")
            ]
        );
        assert_eq!(
            handedness_choices()
                .iter()
                .map(|(label, value)| { (label.as_str(), value.as_str().unwrap_or_default()) })
                .collect::<Vec<_>>(),
            [("Left", "left"), ("Right", "right")]
        );
        let system = json!({"up":"+Z", "handedness":"right"});
        assert_eq!(current_up(&system), "+Z");
        assert_eq!(current_handedness(&system), "Right");
        assert_eq!(up_value(&system), json!("+Z"));
        assert_eq!(handedness_value(&system), json!("right"));
        for missing in [Value::Null, json!({})] {
            assert_eq!(current_up(&missing), "+Y");
            assert_eq!(current_handedness(&missing), "Left");
            assert_eq!(up_value(&missing), json!("+Y"));
            assert_eq!(handedness_value(&missing), json!("left"));
        }
        assert_eq!(
            handedness_value(&json!({"up":"+Y", "handedness":"other"})),
            json!("left")
        );
        assert_eq!(
            up_value(&json!({"up":"diagonal", "handedness":"left"})),
            json!("+Y")
        );
        // Choosing an axis keeps the current handedness, and vice versa.
        assert_eq!(
            coordinate_edit("scene-id", &json!("+Z"), &json!("right")),
            json!({
                "kind":"set_scene_coordinate_system", "scene":"scene-id",
                "up":"+Z", "handedness":"right",
            })
        );
        let component = json!({"kind":"scene", "scene_id":"s", "coordinate_system":system});
        assert_eq!(subtitle(&component, &Value::Null), "+Z up · right-handed");
        let plain = json!({"kind":"scene", "scene_id":"s"});
        assert_eq!(subtitle(&plain, &Value::Null), "+Y up · left-handed");
    }
}
