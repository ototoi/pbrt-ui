//! The node editor's background
//! create. Pure helpers, testable without a window: the menu rows the
//! same backend schema the Contents ▸ New menu builds from, the one
//! `create_resource` edit aimed at the kind's default folder, the graph
//! the editor draws (the root's upstream graph plus the ids created
//! here), when such an id's resource is gone, and the
//! graph-units point of the click that opened the menu. The editor state
//! that holds the ids themselves is never saved — like node positions.
use super::resource_menu;
use serde_json::{Value, json};
use std::collections::HashSet;

/// The background menu's implementation lists — the same three the
/// Contents ▸ New menu builds (`resource_menu::implementations`, so the
/// two menus cannot drift): `New Material ▸ …` and
/// `New Texture ▸ Float ▸ … / Spectrum ▸ …`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CreateMenu {
    pub material: Vec<String>,
    pub texture_float: Vec<String>,
    pub texture_spectrum: Vec<String>,
}

/// The menu's rows from the backend's static summary
/// (`{"material": ..., "texture_float": ..., "texture_spectrum": ...}`);
/// a summary that never arrived lists nothing.
pub fn menu_rows(schema: &Value) -> CreateMenu {
    CreateMenu {
        material: resource_menu::implementations(&schema["material"]),
        texture_float: resource_menu::implementations(&schema["texture_float"]),
        texture_spectrum: resource_menu::implementations(&schema["texture_spectrum"]),
    }
}

/// One background creation — the same edit Contents ▸ New applies
/// (`resource_menu::edit`), aimed at the kind's default folder so the
/// editor needs no Contents selection: Materials/ or Textures/.
pub fn create_edit(kind: &str, implementation: &str, value_type: Option<&str>) -> Value {
    resource_menu::edit(
        kind,
        implementation,
        value_type,
        Some(resource_menu::default_folder(kind)),
    )
}

/// The graph the editor lays out and draws: `root`'s upstream graph plus
/// the graphs of the ids created unconnected here (their latest
/// answers). Nodes dedupe by id — the root's copy wins — edges by their
/// `from`/`to`/`parameter`; the message bars (`truncated`, `error`) speak
/// for the root alone.
pub fn merged_graph(root: &Value, extras: &[Value]) -> Value {
    let mut nodes = root["nodes"].as_array().cloned().unwrap_or_default();
    let mut edges = root["edges"].as_array().cloned().unwrap_or_default();
    let mut seen_ids: HashSet<String> = nodes
        .iter()
        .filter_map(|node| node["id"].as_str().map(str::to_owned))
        .collect();
    let mut seen_links: HashSet<(String, String, String)> = edges
        .iter()
        .filter_map(|edge| {
            Some((
                edge["from"].as_str()?.to_owned(),
                edge["to"].as_str()?.to_owned(),
                edge["parameter"].as_str()?.to_owned(),
            ))
        })
        .collect();
    for extra in extras {
        for node in extra["nodes"].as_array().into_iter().flatten() {
            if let Some(id) = node["id"].as_str()
                && seen_ids.insert(id.to_owned())
            {
                nodes.push(node.clone());
            }
        }
        for edge in extra["edges"].as_array().into_iter().flatten() {
            let link = (
                edge["from"].as_str().unwrap_or_default().to_owned(),
                edge["to"].as_str().unwrap_or_default().to_owned(),
                edge["parameter"].as_str().unwrap_or_default().to_owned(),
            );
            if seen_links.insert(link.clone()) {
                edges.push(edge.clone());
            }
        }
    }
    json!({"nodes": nodes, "edges": edges,
           "truncated": root["truncated"].clone(),
           "error": root["error"].clone()})
}

/// Whether `id`'s graph answer says the resource itself is gone — the
/// ask answered `not_found` (an undo of the creation, a delete). Any
/// other error is the retry class — a write raced the ask onto a
/// superseded reference — never a gone resource: the id stays and the
/// next revision's ask re-asks.
pub fn resource_is_gone(answer: &Value) -> bool {
    answer["error_code"].as_str() == Some("not_found")
}

/// The upstream resource displaced by a replacement or disconnect. Keeping
/// the old id in the editor's extra set preserves its card after the root
/// graph stops reaching it.
pub fn displaced_resource(previous: Option<&str>, replacement: Option<&str>) -> Option<String> {
    previous
        .filter(|previous| Some(*previous) != replacement)
        .map(str::to_owned)
}

pub fn resource_source_for_edge(graph: &Value, edge: &Value) -> Option<String> {
    let source = edge["from"].as_str()?;
    let node = graph["nodes"]
        .as_array()?
        .iter()
        .find(|node| node["id"] == source)?;
    matches!(
        node["kind"].as_str(),
        Some("material" | "texture" | "spectrum_data")
    )
    .then(|| source.to_owned())
}

/// The graph-units point of a window-coordinates click — the viewport
/// origin, pan and zoom undone — where a created node lands so it
/// appears under the right-click that asked for it.
pub fn graph_point(
    position: gpui_kit::Point<gpui_kit::Pixels>,
    origin: gpui_kit::Point<gpui_kit::Pixels>,
    pan: (f32, f32),
    zoom: f32,
) -> (f32, f32) {
    (
        (f32::from(position.x - origin.x) - pan.0) / zoom,
        (f32::from(position.y - origin.y) - pan.1) / zoom,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, px};
    use serde_json::json;

    /// Test graphs in the wire's shape (`graph_layout`'s fixture style).
    fn node(id: &str, inputs: &[&str]) -> Value {
        json!({"id":id,
               "inputs":inputs.iter().map(|parameter| json!({"parameter":parameter})).collect::<Vec<_>>()})
    }

    fn edge(from: &str, to: &str, parameter: &str) -> Value {
        json!({"from":from,"to":to,"parameter":parameter})
    }

    #[test]
    fn menu_rows_mirror_the_contents_new_lists() {
        let schema = json!({
            "material": {"implementations": ["diffuse", "conductor"]},
            "texture_float": {"implementations": ["checkerboard", "constant"]},
            "texture_spectrum": {"implementations": ["imagemap", "mix"]},
            "medium": {"implementations": ["homogeneous"]},
        });
        let rows = menu_rows(&schema);
        assert_eq!(rows.material, ["diffuse", "conductor"]);
        assert_eq!(rows.texture_float, ["checkerboard", "constant"]);
        assert_eq!(rows.texture_spectrum, ["imagemap", "mix"]);
    }

    #[test]
    fn a_schema_that_never_arrived_lists_nothing() {
        let rows = menu_rows(&Value::Null);
        assert_eq!(rows, CreateMenu::default());
        assert_eq!(
            menu_rows(&json!({"material": json!({})})),
            CreateMenu::default()
        );
    }

    #[test]
    fn creations_carry_the_kinds_default_folder() {
        let texture = create_edit("texture", "checkerboard", Some("float"));
        assert_eq!(texture["kind"], "create_resource");
        assert_eq!(texture["resource_kind"], "texture");
        assert_eq!(texture["implementation"], "checkerboard");
        assert_eq!(texture["texture_value_type"], "float");
        assert_eq!(texture["folder"], "Textures");
        assert_eq!(texture["label"], "checkerboard");
        let material = create_edit("material", "diffuse", None);
        assert_eq!(material["resource_kind"], "material");
        assert_eq!(material["folder"], "Materials");
        assert!(material["texture_value_type"].is_null());
    }

    #[test]
    fn a_merged_graph_draws_an_unconnected_id_and_dedupes_a_shared_one() {
        // wall ← tint; apart from them, "fresh" was created here and
        // then tint was wired into its slot, so tint rides both graphs.
        let root = json!({
            "nodes":[node("wall", &["reflectance"]), node("tint", &["tex1"])],
            "edges":[edge("tint", "wall", "reflectance")],
        });
        let extra = json!({
            "nodes":[node("fresh", &["tex"]), node("tint", &["tex1"])],
            "edges":[edge("tint", "fresh", "tex")],
        });
        // No extras: the root as it is.
        assert_eq!(
            merged_graph(&root, &[])["nodes"].as_array().map(Vec::len),
            Some(2)
        );
        let merged = merged_graph(&root, std::slice::from_ref(&extra));
        let id = |index: usize| merged["nodes"][index]["id"].clone();
        assert_eq!(
            merged["nodes"].as_array().map(Vec::len),
            Some(3),
            "the shared tint dedupes; fresh is new"
        );
        assert_eq!(id(0), "wall");
        assert_eq!(id(1), "tint");
        assert_eq!(id(2), "fresh");
        // Both edges survive the merge — cards, the drop check and the
        // drawing all read them — under one dedupe by their triple.
        let links: HashSet<(String, String, String)> = merged["edges"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|edge| {
                Some((
                    edge["from"].as_str()?.to_owned(),
                    edge["to"].as_str()?.to_owned(),
                    edge["parameter"].as_str()?.to_owned(),
                ))
            })
            .collect();
        assert_eq!(
            links,
            [
                (
                    "tint".to_owned(),
                    "wall".to_owned(),
                    "reflectance".to_owned()
                ),
                ("tint".to_owned(), "fresh".to_owned(), "tex".to_owned()),
            ]
            .into()
        );
    }

    #[test]
    fn a_merged_graph_says_what_says_the_root_and_not_the_extras() {
        let root = json!({
            "nodes":[node("wall", &[])],
            "edges":[],
            "truncated": true,
            "error": "upstream incomplete",
        });
        let extra = json!({"nodes":[node("fresh", &[])], "edges":[]});
        let merged = merged_graph(&root, &[extra]);
        assert_eq!(merged["truncated"], true);
        assert_eq!(merged["error"], "upstream incomplete");
        // An empty-rooted view still draws
        // the created node.
        let fresh_only = merged_graph(&json!({}), &[json!({"nodes":[node("fresh", &[])]})]);
        assert_eq!(fresh_only["nodes"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn only_a_not_found_answer_says_the_resource_is_gone() {
        // The ask for an undo'd or deleted resource answers not_found…
        assert!(resource_is_gone(&json!({
            "error": "resource not found", "error_code": "not_found"
        })));
        // …while a revision that moved under the ask is the retry class:
        // dropping the node there would erase a live creation (a write
        // raced the ask onto a superseded reference — the next revision
        // re-asks), so neither race answer may count as gone.
        assert!(!resource_is_gone(
            &json!({"error": "revision conflict", "error_code": "revision_conflict"})
        ));
        assert!(!resource_is_gone(
            &json!({"error": "stale project", "error_code": "stale_project"})
        ));
        // A graph — answered, or an answer without a code — keeps the
        // id too.
        assert!(!resource_is_gone(&json!({"nodes": [], "edges": []})));
        assert!(!resource_is_gone(&json!({"error": "resource not found"})));
        assert!(!resource_is_gone(&json!({})));
    }

    #[test]
    fn displaced_resources_stay_only_when_a_link_is_replaced_or_disconnected() {
        assert_eq!(
            displaced_resource(Some("old"), Some("new")),
            Some("old".into())
        );
        assert_eq!(displaced_resource(Some("old"), None), Some("old".into()));
        assert_eq!(displaced_resource(Some("same"), Some("same")), None);
        assert_eq!(displaced_resource(None, Some("new")), None);
    }

    #[test]
    fn only_resource_nodes_from_a_displaced_edge_are_retained() {
        let graph = json!({"nodes":[
            {"id":"tint","kind":"texture"},
            {"id":"wall:reflectance","kind":"spectrum_value"}
        ]});
        assert_eq!(
            resource_source_for_edge(&graph, &json!({"from":"tint"})),
            Some("tint".into())
        );
        assert_eq!(
            resource_source_for_edge(&graph, &json!({"from":"wall:reflectance"})),
            None
        );
    }

    #[test]
    fn the_click_point_is_the_position_with_pan_origin_and_zoom_undone() {
        // Hand-worked: / 2 = 35, / 2 = 15.
        assert_eq!(
            graph_point(
                point(px(120.), px(80.)),
                point(px(10.), px(20.)),
                (40., 30.),
                2.
            ),
            (35., 15.)
        );
        assert_eq!(
            graph_point(point(px(50.), px(25.)), point(px(0.), px(0.)), (0., 0.), 1.),
            (50., 25.)
        );
    }
}
