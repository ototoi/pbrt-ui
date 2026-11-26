//! Project Inspector: a node as Unity-style component boxes, or the item
//! selected in Contents. Viewer documents show everything read-only.
use crate::backend::{Command, Follow, PreviewKind};
use crate::editor::component_menu;
use crate::editor::components::{self, Folded};
use crate::editor::edit_feedback::EditFeedback;
use crate::editor::menu::{
    ContentsOpen, CreatePreset, OpenInEditor, RevealDefinition, RevealItem, RevealNode,
    SaveProject, UnpackObjectInstance,
};
use crate::editor::node_editor::swatch_element;
use crate::editor::{EditorPanel, Session, editor_panels, spectrum_editor};
use crate::editor::{parameter_field, parameter_rows, transform_form::TransformForm};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

fn muted() -> Rgba {
    rgb(0x9098a6)
}

fn row(label: impl Into<SharedString>, value: impl IntoElement) -> Div {
    div()
        .flex()
        .gap_2()
        .py(px(2.))
        .child(
            div()
                .w(px(92.))
                .flex_shrink_0()
                .text_color(muted())
                .child(label.into()),
        )
        .child(div().flex_1().min_w_0().child(value))
}

struct HeaderDetails {
    lines: Vec<String>,
    open_in_editor: Option<String>,
    read_only: bool,
    active_toggle: Option<(String, bool)>,
    node_kind: Option<String>,
}

fn inspector_node_menu(mut menu: PopupMenu, kind: &str, read_only: bool) -> PopupMenu {
    for item in crate::editor::hierarchy::node_menu_items(kind, read_only) {
        let action = item.action;
        menu =
            menu.item(
                PopupMenuItem::new(item.label).on_click(move |_, window, cx| match action {
                    crate::editor::hierarchy::NodeMenuAction::Rename => window
                        .dispatch_action(Box::new(crate::editor::menu::RenameHierarchyNode), cx),
                    crate::editor::hierarchy::NodeMenuAction::Duplicate => {
                        window.dispatch_action(Box::new(crate::editor::menu::DuplicateNode), cx)
                    }
                    crate::editor::hierarchy::NodeMenuAction::Delete => {
                        window.dispatch_action(Box::new(crate::editor::menu::DeleteNode), cx)
                    }
                    crate::editor::hierarchy::NodeMenuAction::MakeObjectDefinition => window
                        .dispatch_action(Box::new(crate::editor::menu::MakeObjectDefinition), cx),
                    crate::editor::hierarchy::NodeMenuAction::UnpackInstance => window
                        .dispatch_action(Box::new(crate::editor::menu::UnpackObjectInstance), cx),
                    crate::editor::hierarchy::NodeMenuAction::FrameSelection => {
                        window.dispatch_action(Box::new(crate::editor::FrameSelected), cx)
                    }
                }),
            );
    }
    menu
}

/// Send one edit; the Viewer never gets here because its forms are hidden.
fn apply(session: &Entity<Session>, form: &str, edit: Value, cx: &mut App) {
    apply_all(session, form, vec![edit], cx)
}

/// Send edits as one transaction (one undo step).
fn apply_all(session: &Entity<Session>, form: &str, edits: Vec<Value>, cx: &mut App) {
    session.update(cx, |session, cx| {
        let project = session.project_value();
        if project.is_object() && !session.document().is_read_only() {
            session.edit_feedback.submitted(form);
            session.send(
                Command::Apply {
                    project,
                    edits,
                    follow: Follow::Nothing,
                    form: Some(form.to_owned()),
                },
                cx,
            );
        }
    });
}

/// One where-used row's label: its source's name and its via.
pub(crate) fn reference_label(row: &Value) -> String {
    let via = row["via"].as_str().unwrap_or_default().to_owned();
    let name = match row["kind"].as_str() {
        Some("node") => format!(
            "{} / {}",
            row["scene"]["label"].as_str().unwrap_or_default(),
            row["node"]["label"].as_str().unwrap_or_default(),
        ),
        Some("definition_node") => format!(
            "{} / {}",
            row["definition"]["label"].as_str().unwrap_or_default(),
            row["node"]["label"].as_str().unwrap_or_default(),
        ),
        _ => row["resource"]["label"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    };
    if via.is_empty() {
        name
    } else {
        format!("{name} ({via})")
    }
}

/// The reveal one where-used row dispatches: a scene node opens its
/// scene and reveals it in the Hierarchy; a definition's inner node and a
/// resource open their item in Contents — a definition's inner nodes have
/// no place in the Hierarchy's scene trees.
fn reference_reveal(row: &Value) -> Box<dyn Action> {
    match row["kind"].as_str() {
        Some("node") => Box::new(RevealNode {
            scene: row["scene"]["id"].as_str().unwrap_or_default().to_owned(),
            node_id: row["node"]["id"].as_str().unwrap_or_default().to_owned(),
            path: row["node"]["path"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect(),
        }),
        Some("definition_node") => Box::new(RevealItem {
            id: row["definition"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        }),
        _ => Box::new(RevealItem {
            id: row["resource"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        }),
    }
}

/// A link: selects the item in Contents and reveals its folder.
fn link(id: &str, label: String) -> AnyElement {
    let id = id.to_owned();
    div()
        .id(SharedString::from(format!("link-{id}")))
        .text_color(rgb(0x7fb2f0))
        .cursor_pointer()
        .hover(|style| style.underline())
        .child(label)
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(RevealItem { id: id.clone() }), cx)
        })
        .into_any_element()
}

/// A message under a field: the core's rejection or another edit's change.
fn feedback_line(message: String) -> Div {
    div().text_color(rgb(0xd99a74)).child(message)
}

/// The shape's alpha mask rows, shown in their own component box.
fn alpha_mask_row(name: &str) -> bool {
    matches!(name, "alpha" | "shadowalpha")
}

fn matrix_display_rows(matrix: &Value) -> Vec<AnyElement> {
    components::matrix_rows(matrix)
        .into_iter()
        .enumerate()
        .map(|(index, values)| {
            row(
                format!("Row {index}"),
                div()
                    .font_family("DejaVu Sans Mono")
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(values),
            )
            .into_any_element()
        })
        .collect()
}

/// Whether the transform box can show a TRS form — a single local
/// needs its own `trs`, an animated one a `trs` for every endpoint that
/// carries a matrix.
fn local_has_trs(local: &Value) -> bool {
    if let Some(samples) = local["samples"].as_object() {
        let endpoints = [&samples["start"], &samples["end"]]
            .into_iter()
            .filter(|sample| sample["matrix"].is_array())
            .collect::<Vec<_>>();
        !endpoints.is_empty() && endpoints.iter().all(|sample| sample["trs"].is_object())
    } else {
        local["trs"].is_object()
    }
}

/// The Transform component box's header switch —
/// row, drawn as the TRS / Matrix 4×4 icon buttons beside the ⋮ menu.
#[derive(Debug, PartialEq)]
struct TransformHeaderControl {
    /// The display the box shows now: the session's choice, or the
    /// matrix when no TRS exists.
    matrix: bool,
    /// Whether a TRS display exists at all; the TRS button needs this.
    has_trs: bool,
    /// The form whose Apply the ⋮ menu exposes: `None` when the
    /// box has no editable form (read-only, or an animated transform).
    form: Option<Entity<TransformForm>>,
}

/// The header switch for a transform component, or `None` where the
/// Mode row never drew either — no transform kind, or nothing
/// displayable in its local.
fn transform_header_control(
    component: &Value,
    session_matrix: bool,
    form: Option<Entity<TransformForm>>,
) -> Option<TransformHeaderControl> {
    let local = &component["local"];
    (component["kind"] == "transform"
        && (local["matrix"].is_array() || local["samples"].is_object()))
    .then(|| {
        let has_trs = local_has_trs(local);
        TransformHeaderControl {
            matrix: session_matrix || !has_trs,
            has_trs,
            form,
        }
    })
}

/// Return whether an item's Inspector page builds
/// typed parameter rows for its target. A spectrum data resource's
/// page shows a read-only summary instead — its wavelengths / values
/// tables never become fields here or in `prepare_inspector` (the
/// Spectrum Data editor owns the editing).
pub(super) fn item_builds_parameter_rows(item: &Value) -> bool {
    item["resource_kind"] != "spectrum_data"
}

/// Format the Samples line of a spectrum data resource's
/// read-only summary — the sample count, the first-to-last wavelength
/// range and the highest value, spelled the way the Spectrum Data
/// editor renders its cells (`cell_number`), unpadded (not
/// `curve_axes`). `None` (the table still on its way) shows `…`; an
/// `{error}` payload — or a table the editor cannot read — shows its
/// message. A fetched table parses through `rows_from_table`, the
/// reader the Spectrum Data editor loads with.
fn spectrum_samples_text(table: Option<&Value>) -> String {
    match table.map(spectrum_editor::rows_from_table) {
        None => "…".to_owned(),
        Some(Err(message)) => message,
        Some(Ok(loaded)) => {
            let curve = &loaded.curve;
            let count = curve.len();
            let samples = if count == 1 { "sample" } else { "samples" };
            let Some(&(first, _)) = curve.first() else {
                return format!("{count} {samples}");
            };
            let last = curve[count - 1].0;
            let up_to = curve.iter().map(|&(_, value)| value).fold(0., f64::max);
            format!(
                "{count} {samples} · {} – {} nm · values up to {}",
                spectrum_editor::cell_number(first),
                spectrum_editor::cell_number(last),
                spectrum_editor::cell_number(up_to)
            )
        }
    }
}

/// Describe what a component box adds beyond the standard box: the ⋮
/// menu's removal edits, and for a Transform box the header's TRS /
/// Matrix 4×4 switch.
struct BoxChrome {
    remove: Option<Vec<Value>>,
    transform: Option<TransformHeaderControl>,
    camera_align: Option<Value>,
}

impl EditorPanel {
    /// The typed fields built for one target whose names `keep` accepts,
    /// or read-only rows.
    fn parameters(
        &self,
        target_id: &str,
        page: &Value,
        read_only: bool,
        keep: &dyn Fn(&str) -> bool,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let forms = self
            .parameter_forms
            .iter()
            .filter(|(_, target, form)| target == target_id && keep(form.read(cx).name()))
            .map(|(_, _, form)| {
                (
                    form.read(cx).name().to_owned(),
                    form.clone().into_any_element(),
                )
            })
            .collect::<Vec<_>>();
        let rows = if read_only || forms.is_empty() {
            parameter_field::read_only_rows(page, keep)
        } else {
            forms
        };
        self.with_metal_colour(rows, page, target_id, cx)
    }

    /// A conductor or coatedconductor page places the read-only
    /// Metal colour row before its eta (or conductor.eta) row — the wire's
    /// conductor-role answer, one element on the page for the metal's
    /// normal-incidence F0, whose tooltip spells the sides (and r4's
    /// refusal of a reflectance beside eta/k). Anything else shows the
    /// rows it was given.
    fn with_metal_colour(
        &self,
        rows: Vec<(String, AnyElement)>,
        page: &Value,
        target_id: &str,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let mut ask = |parameter: &str| {
            parameter_field::metal_colour_row(
                &self.session,
                &serde_json::json!({"kind":"resource","id":target_id}),
                parameter,
                cx,
            )
            .into_any_element()
        };
        let insert_before = |implementation: &str| {
            match implementation {
                "conductor" => vec!["conductor.eta", "eta"],
                "coatedconductor" => vec!["conductor.eta", "conductor.k", "eta", "k"],
                _ => vec![],
            }
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        };
        let implementation = page["implementation"].as_str().unwrap_or_default();
        let order = insert_before(implementation);
        let Some(ask_row) = order.first() else {
            return rows.into_iter().map(|(_, element)| element).collect();
        };
        let mut metal = Some(ask(ask_row));
        let mut out = Vec::with_capacity(rows.len() + 1);
        let mut placed = false;
        for (name, element) in rows {
            if !placed
                && order.contains(&name)
                && let Some(metal) = metal.take()
            {
                out.push(metal);
                placed = true;
            }
            out.push(element);
        }
        if let Some(metal) = metal {
            out.push(metal);
        }
        out
    }

    /// The header's TRS / Matrix 4×4 buttons — the display-only switch
    /// the removed Mode row made, the active one highlighted and each
    /// button named by its tooltip. A click stops propagation so the
    /// surrounding header does not also fold the box.
    fn transform_header_buttons(&self, control: &TransformHeaderControl) -> Div {
        let session = self.session.clone();
        let mode_button =
            |id: &'static str, icon: assets::IconName, name: &'static str, show_matrix: bool| {
                let active = control.matrix == show_matrix;
                let session = session.clone();
                Button::new(id)
                    .icon(icon)
                    .xsmall()
                    // `toggled` is the accessible pressed state; the
                    // selected fill highlights the active display.
                    .when(active, |button| button.primary())
                    .toggled(active)
                    .tooltip(name)
                    .accessibility_label(name)
                    .disabled(!show_matrix && !control.has_trs)
                    .on_click(move |_, _, cx| {
                        session.update(cx, |session, cx| {
                            session.transform_matrix = show_matrix;
                            cx.notify();
                        });
                        // The clickable header folds the box on click; a
                        // mode switch must not.
                        cx.stop_propagation();
                    })
            };
        div()
            .flex()
            .items_center()
            .child(mode_button(
                "transform-mode-trs",
                assets::IconName::Move3d,
                "TRS",
                false,
            ))
            .child(mode_button(
                "transform-mode-matrix",
                assets::IconName::Grid3x3,
                "Matrix 4×4",
                true,
            ))
    }

    fn component_box(
        &self,
        component: &Value,
        body: Vec<AnyElement>,
        source: String,
        folded: &Folded,
        items: &Value,
        chrome: BoxChrome,
    ) -> AnyElement {
        let BoxChrome {
            remove,
            transform,
            camera_align,
        } = chrome;
        let kind = component["kind"].as_str().unwrap_or("component").to_owned();
        let (title, icon) = components::title(&kind);
        let closed = folded.is_folded(&kind);
        let subtitle = components::subtitle(component, items);
        let json_text = serde_json::to_string_pretty(component).unwrap_or_default();
        let session = self.session.clone();
        let menu_session = self.session.clone();
        let toggle_kind = kind.clone();
        let apply_form = transform.as_ref().and_then(|control| control.form.clone());
        div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgb(0x3a3d45))
            .rounded_md()
            .child(
                div()
                    .id(SharedString::from(format!("component-{kind}")))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .h(px(26.))
                    .bg(rgb(0x2d3036))
                    .cursor_pointer()
                    .child(
                        Icon::new(if closed {
                            assets::IconName::ChevronRight
                        } else {
                            assets::IconName::ChevronDown
                        })
                        .size(px(12.)),
                    )
                    .child(Icon::new(icon).size(px(14.)).text_color(rgb(0xaeb5c4)))
                    .child(div().font_weight(FontWeight::BOLD).child(title))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(muted())
                            .child(if subtitle.is_empty() {
                                String::new()
                            } else {
                                format!("· {subtitle}")
                            }),
                    )
                    .on_click(move |_, _, cx| {
                        session.update(cx, |session, cx| {
                            session.toggle_folded(&toggle_kind);
                            cx.notify();
                        })
                    })
                    .when_some(
                        transform.as_ref().map(|c| self.transform_header_buttons(c)),
                        |header, buttons| header.child(buttons),
                    )
                    .child(
                        Button::new(SharedString::from(format!("component-menu-{kind}")))
                            .icon(assets::IconName::EllipsisVertical)
                            .xsmall()
                            .accessibility_label("Component actions")
                            .dropdown_menu(move |menu, _, cx| {
                                let text = json_text.clone();
                                let line = source.clone();
                                let menu_session = menu_session.clone();
                                let mut menu = menu;
                                // Apply Transform heads the menu, the
                                // mouse path Enter already offers; enabled
                                // only while the form holds unsent edits.
                                // No form draws no
                                // item at all.
                                if let Some(form) = apply_form.clone() {
                                    let unsent = form.read(cx).has_unsent_changes(cx);
                                    menu = menu
                                        .item(
                                            PopupMenuItem::new("Apply Transform")
                                                .disabled(!unsent)
                                                .on_click(move |_, _, cx| {
                                                    form.update(cx, |form, cx| form.apply(cx));
                                                }),
                                        )
                                        .separator();
                                }
                                if let Some(edits) = remove.clone() {
                                    let session = menu_session.clone();
                                    menu = menu
                                        .item(PopupMenuItem::new("Remove Component").on_click(
                                            move |_, _, cx| {
                                                apply_all(
                                                    &session,
                                                    "inspector-remove",
                                                    edits.clone(),
                                                    cx,
                                                )
                                            },
                                        ))
                                        .separator();
                                }
                                if let Some(align) = camera_align.clone() {
                                    let session = menu_session.clone();
                                    let camera_id = align["camera_id"].as_str().unwrap_or_default().to_owned();
                                    menu = menu.separator().item(
                                        PopupMenuItem::new("Align Camera to View").on_click(
                                            move |_, _, cx| {
                                                session.update(cx, |session, cx| {
                                                    let (Some(_project_ref), Some(scene_id), Some(camera)) = (
                                                        session.project.context.project.clone(),
                                                        session.project.active_scene,
                                                        session.view_camera,
                                                    ) else { return; };
                                                    let Ok(coordinate_system) = serde_json::from_value(json!({
                                                        "up": session.project.active_up.unwrap_or(protocol::project::UpAxis::PosY),
                                                        "handedness": session.project.active_handedness.unwrap_or(protocol::project::Handedness::Left),
                                                    })) else { return; };
                                                    let image_size = session.image_size;
                                                    session.send(Command::AlignCamera {
                                                        project: session.project_value(),
                                                        scene_id: scene_id.to_string(),
                                                        camera_id: camera_id.clone(),
                                                        coordinate_system,
                                                        view: project::presets::PresetView { position: camera.eye, look_at: camera.target },
                                                        view_fov_y_degrees: camera.fov_y_degrees,
                                                        viewport_aspect: image_size[0].max(1) as f64 / image_size[1].max(1) as f64,
                                                    }, cx);
                                                })
                                            },
                                        ),
                                    );
                                }
                                menu.item(PopupMenuItem::new("Copy Value").on_click(
                                    move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            text.clone(),
                                        ))
                                    },
                                ))
                                .item(
                                    PopupMenuItem::new("Show Source in Console").on_click(
                                        move |_, _, cx| {
                                            let line = line.clone();
                                            menu_session.update(cx, |session, cx| {
                                                session.messages.push(line);
                                                cx.notify();
                                            })
                                        },
                                    ),
                                )
                            }),
                    ),
            )
            .when(!closed, |this| {
                this.child(div().flex().flex_col().gap_1().px_2().py_1().children(body))
            })
            .into_any_element()
    }

    /// One selector button whose menu sends a `set_local_binding` edit.
    fn selector(
        &self,
        id: String,
        current: String,
        choices: Vec<(String, Value)>,
        edit: impl Fn(&Value) -> Value + 'static,
        read_only: bool,
        feedback: &EditFeedback,
    ) -> AnyElement {
        if read_only {
            return div().child(current).into_any_element();
        }
        let session = self.session.clone();
        let edit = std::rc::Rc::new(edit);
        let message = feedback.message(&id).map(str::to_owned);
        let form = id.clone();
        let button = Button::new(SharedString::from(id))
            .label(current)
            .xsmall()
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;
                for (label, value) in &choices {
                    let session = session.clone();
                    let edit = edit.clone();
                    let value = value.clone();
                    let form = form.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label.clone())
                            .on_click(move |_, _, cx| apply(&session, &form, edit(&value), cx)),
                    );
                }
                menu
            });
        div()
            .flex()
            .flex_col()
            .child(button)
            .when_some(message, |this, message| this.child(feedback_line(message)))
            .into_any_element()
    }

    fn material_selector(
        &self,
        node_id: String,
        current: String,
        choices: Vec<(String, Value)>,
        implementations: Vec<String>,
        read_only: bool,
        feedback: &EditFeedback,
    ) -> AnyElement {
        if read_only {
            return div().child(current).into_any_element();
        }
        let session = self.session.clone();
        let form = format!("material-{node_id}");
        let message = feedback.message(&form).map(str::to_owned);
        let menu = Button::new(SharedString::from(form.clone()))
            .label(current)
            .xsmall()
            .dropdown_menu(move |menu, window, cx| {
                let mut menu = menu;
                for (label, binding) in &choices {
                    let session = session.clone();
                    let form = form.clone();
                    let binding = binding.clone();
                    let node = node_id.clone();
                    menu =
                        menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, _, cx| {
                            apply(
                                &session,
                                &form,
                                components::material_edit(&node, &binding),
                                cx,
                            )
                        }));
                }
                if !implementations.is_empty() {
                    let session = session.clone();
                    let form = form.clone();
                    let node = node_id.clone();
                    let implementations = implementations.clone();
                    menu = menu.submenu("New Material", window, cx, move |menu, _, _| {
                        let mut menu = menu;
                        for implementation in &implementations {
                            let session = session.clone();
                            let form = form.clone();
                            let node = node.clone();
                            let implementation = implementation.clone();
                            menu = menu.item(PopupMenuItem::new(implementation.clone()).on_click(
                                move |_, _, cx| {
                                    let id = uuid::Uuid::new_v4().to_string();
                                    apply_all(
                                        &session,
                                        &form,
                                        components::new_material_edits(&node, &implementation, &id),
                                        cx,
                                    );
                                },
                            ));
                        }
                        menu
                    });
                }
                menu
            });
        div()
            .flex()
            .flex_col()
            .child(menu)
            .when_some(message, |this, message| this.child(feedback_line(message)))
            .into_any_element()
    }

    /// Build the implementation (Type) row of one component box. Its selector
    /// asks the core for a plan so the
    /// window can confirm removed or unknown parameters before applying.
    /// Hidden in the Viewer and for boxes without candidates.
    fn implementation_row(
        &self,
        component: &Value,
        target: &Value,
        schema: &Value,
        read_only: bool,
        feedback: &EditFeedback,
    ) -> Option<AnyElement> {
        let selector = parameter_field::implementation_selector(
            &self.session,
            component,
            target,
            schema,
            read_only,
            feedback,
        )?;
        // "Type", as pbrt spells it: "Implementation"
        // does not fit the label column.
        Some(row("Type", selector).into_any_element())
    }

    /// Build the read-only summary a spectrum data resource's
    /// box shows instead of its wavelengths / values parameter rows —
    /// the approximate chromaticity colour (the swatch and role the
    /// node editor's spectrum node, the Contents thumbnail and the
    /// Spectrum Data editor share, hollow while the answer is on its
    /// way) with the answer's `source` spelled beside it, and one
    /// Samples line from the whole-table cache the Spectrum Data editor
    /// loads through. The "Edit Spectrum…" button below dispatches the
    /// header pen button's action, shown only where that button works
    /// (`editor_for`: project resources, not builtins) — enabled in a
    /// read-only Viewer, exactly like the header.
    fn spectrum_data_summary_body(
        &self,
        target: &str,
        item: &Value,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let swatch = self.session.update(cx, |session, cx| {
            session.spectrum_swatch(target, "chromaticity", cx)
        });
        let source = swatch
            .as_ref()
            .and_then(|answer| answer["source"].as_str())
            .map(str::to_owned);
        let colour = row(
            "Colour",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(swatch_element(swatch, 14.))
                .when_some(source, |this, source| {
                    this.child(div().min_w_0().truncate().text_color(muted()).child(source))
                }),
        )
        .into_any_element();
        let table = self
            .session
            .update(cx, |session, cx| session.spectrum_table(target, cx));
        let samples = row(
            "Samples",
            div().child(spectrum_samples_text(table.as_ref())),
        )
        .into_any_element();
        let mut body = vec![colour, samples];
        if editor_panels::editor_for(item).is_some()
            && let Some(id) = item["id"].as_str().map(str::to_owned)
        {
            body.push(
                Button::new("inspector-edit-spectrum")
                    .label("Edit Spectrum…")
                    .xsmall()
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(OpenInEditor { id: id.clone() }), cx)
                    })
                    .into_any_element(),
            );
        }
        body
    }

    /// Display the local transform as TRS or a 4×4 matrix. The switch only
    /// changes the display; TRS is disabled, with the reason, when the matrix
    /// has none.
    fn local_transform(&self, component: &Value, read_only: bool, cx: &App) -> Vec<AnyElement> {
        let local = &component["local"];
        let samples = local["samples"].as_object().map(|_| &local["samples"]);
        let mut body = Vec::new();
        if !local["matrix"].is_array() && samples.is_none() {
            let reason = components::transform_reason(&local["matrix_reason"])
                .map(|reason| format!("The local transform {reason}"))
                .unwrap_or_else(|| "A single local transform is unavailable".into());
            body.push(div().text_color(muted()).child(reason).into_any_element());
            return body;
        }
        let editable = component["editable"] != false && !read_only;
        if !editable {
            let reason = if read_only {
                "This project is read-only"
            } else {
                component["read_only_reason"]
                    .as_str()
                    .unwrap_or("This transform is read-only")
            };
            body.push(
                div()
                    .text_color(muted())
                    .child(reason.to_owned())
                    .into_any_element(),
            );
        }
        // The TRS / Matrix 4×4 switch moved to the component header
        // `matrix` still owns the display.
        let has_trs = local_has_trs(local);
        let matrix = self.session.read(cx).transform_matrix || !has_trs;
        if samples.is_none()
            && let Some(reason) = components::transform_reason(&local["trs_reason"])
        {
            body.push(
                div()
                    .text_color(muted())
                    .child(format!("TRS unavailable: matrix {reason}"))
                    .into_any_element(),
            );
        }
        let form = self
            .transform_forms
            .first()
            .map(|(_, _, form)| form.clone())
            .filter(|_| editable && local["matrix"].is_array() && samples.is_none());
        if let Some(form) = form {
            body.push(form.into_any_element());
        } else if let Some(samples) = samples {
            let time_text = |index| {
                samples["times"][index]
                    .as_f64()
                    .map(|time| time.to_string())
                    .unwrap_or_else(|| "?".into())
            };
            let paired = |left: String, right: String| {
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(left))
                    .child(div().flex_1().min_w_0().child(right))
            };
            body.push(
                row(
                    "",
                    paired(
                        format!("Start · {}", time_text(0)),
                        format!("End · {}", time_text(1)),
                    )
                    .text_color(muted()),
                )
                .into_any_element(),
            );
            if let Some((_, form)) = &self.animated_transform_form {
                body.push(form.clone().into_any_element());
            }
        } else if matrix {
            body.extend(matrix_display_rows(&local["matrix"]));
        } else {
            for (label, text) in components::trs_rows(&local["trs"]) {
                body.push(row(label, div().child(text)).into_any_element());
            }
        }
        body
    }

    fn component_body(
        &self,
        component: &Value,
        details: &Value,
        schema: &Value,
        read_only: bool,
        feedback: &EditFeedback,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let items = &details["items"];
        let node = &details["node"];
        let node_id = node["node_id"].as_str().unwrap_or_default().to_owned();
        let mut body = Vec::new();
        match component["kind"].as_str().unwrap_or("") {
            "render_settings" => {
                let cameras = component["cameras"].as_array().cloned().unwrap_or_default();
                let counts = cameras
                    .iter()
                    .filter_map(|camera| camera["label"].as_str())
                    .fold(
                        std::collections::BTreeMap::<String, usize>::new(),
                        |mut counts, label| {
                            *counts.entry(label.to_owned()).or_default() += 1;
                            counts
                        },
                    );
                let mut choices = vec![("None".to_owned(), Value::Null)];
                for camera in &cameras {
                    let Some(id) = camera["id"].as_str() else {
                        continue;
                    };
                    let label = camera["label"].as_str().unwrap_or("Camera");
                    let display = if counts.get(label).copied().unwrap_or_default() > 1 {
                        format!("{label} ({})", &id[..8.min(id.len())])
                    } else {
                        label.to_owned()
                    };
                    choices.push((display, json!(id)));
                }
                let selected = component["camera"].as_str().unwrap_or_default();
                let current = choices
                    .iter()
                    .find(|(_, value)| value.as_str().unwrap_or_default() == selected)
                    .map(|(label, _)| label.clone())
                    .unwrap_or_else(|| "None".into());
                let scene = component["scene_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                body.push(row("Camera", self.selector(
                    format!("render-camera-{scene}"), current, choices,
                    move |camera| json!({"kind":"set_render_camera","scene":scene,"camera":camera}),
                    read_only, feedback,
                )).into_any_element());
            }
            "scene" => {
                // Show the scene's coordinate system using the Up and
                // handedness display defaults; edits change metadata only.
                let scene = component["scene_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let system = component["coordinate_system"].clone();
                let up = components::up_value(&system);
                let handedness = components::handedness_value(&system);
                let scene_up = scene.clone();
                let up_handover = handedness.clone();
                body.push(
                    row(
                        "Up",
                        self.selector(
                            format!("scene-up-{scene}"),
                            components::current_up(&system),
                            components::up_choices(),
                            move |choice| {
                                components::coordinate_edit(&scene_up, choice, &up_handover)
                            },
                            read_only,
                            feedback,
                        ),
                    )
                    .into_any_element(),
                );
                let scene_handedness = scene.clone();
                let handedness_up = up.clone();
                body.push(
                    row(
                        "Handedness",
                        self.selector(
                            format!("scene-handedness-{scene}"),
                            components::current_handedness(&system),
                            components::handedness_choices(),
                            move |choice| {
                                components::coordinate_edit(
                                    &scene_handedness,
                                    &handedness_up,
                                    choice,
                                )
                            },
                            read_only,
                            feedback,
                        ),
                    )
                    .into_any_element(),
                );
                body.push(
                    div()
                        .text_color(muted())
                        .child("Metadata only — existing nodes are not moved.")
                        .into_any_element(),
                );
            }
            "transform" => {
                body.extend(self.local_transform(component, read_only, cx));
            }
            "shape" | "light" | "camera" | "area_light" | "film" | "sampler" | "integrator"
            | "pixel_filter" | "accelerator" => {
                let target = component["parameter_target"]["id"]
                    .as_str()
                    .unwrap_or_default();
                let page = if target == node_id {
                    &details["parameters"]
                } else {
                    &details["component_parameters"][target]
                };
                body.extend(self.implementation_row(
                    component,
                    &component["parameter_target"],
                    schema,
                    read_only,
                    feedback,
                ));
                if let Some(asset) = component["mesh_asset"].as_str() {
                    body.push(
                        row("Mesh", link(asset, components::item_name(items, asset)))
                            .into_any_element(),
                    );
                }
                if let Some(statistics) = component["statistics"].as_object() {
                    body.push(
                        row(
                            "Topology",
                            div().child(format!(
                                "{} vertices · {} faces",
                                statistics.get("vertices").cloned().unwrap_or_default(),
                                statistics.get("faces").cloned().unwrap_or_default()
                            )),
                        )
                        .into_any_element(),
                    );
                }
                // A shape's alpha / shadowalpha live in the Alpha Mask box.
                let keep = |name: &str| component["kind"] != "shape" || !alpha_mask_row(name);
                body.extend(self.parameters(target, page, read_only, &keep, cx));
            }
            "alpha_mask" => {
                let target = component["parameter_target"]["id"]
                    .as_str()
                    .unwrap_or_default();
                body.extend(self.parameters(
                    target,
                    &details["parameters"],
                    read_only,
                    &alpha_mask_row,
                    cx,
                ));
            }
            "material" => {
                let current = components::subtitle(component, items);
                if let Some(id) = component["binding"]["value"].as_str() {
                    body.push(
                        row("Bound", link(id, components::item_name(items, id))).into_any_element(),
                    );
                }
                body.push(
                    row(
                        "Material",
                        self.material_selector(
                            node_id.clone(),
                            current,
                            components::material_choices(items),
                            crate::editor::resource_menu::implementations(
                                &self.session.read(cx).resource_schema["material"],
                            ),
                            read_only,
                            feedback,
                        ),
                    )
                    .into_any_element(),
                );
            }
            "medium" if component.get("inside").is_some() || component.get("outside").is_some() => {
                for (side, inside) in [("Inside", true), ("Outside", false)] {
                    let key = if inside { "inside" } else { "outside" };
                    let current = component[key]
                        .as_str()
                        .map(|id| components::item_name(items, id))
                        .unwrap_or_else(|| "None".into());
                    let node = node_id.clone();
                    let medium = component.clone();
                    body.push(
                        row(
                            side,
                            self.selector(
                                format!("medium-{key}-{node_id}"),
                                current,
                                components::medium_choices(items),
                                move |value| components::medium_edit(&node, &medium, inside, value),
                                read_only,
                                feedback,
                            ),
                        )
                        .into_any_element(),
                    );
                }
            }
            "medium" if component.get("binding").is_some() => {
                let current = component["binding"]
                    .as_str()
                    .map(|id| components::item_name(items, id))
                    .unwrap_or_else(|| "None".into());
                let node = node_id.clone();
                let medium = component.clone();
                body.push(
                    row(
                        "Medium",
                        self.selector(
                            format!("camera-medium-{node_id}"),
                            current,
                            components::medium_choices(items),
                            move |value| components::camera_medium_edit(&node, &medium, value),
                            read_only,
                            feedback,
                        ),
                    )
                    .into_any_element(),
                );
            }
            "reverse_orientation" => {
                // Show the effective flag; choosing the other value lets the
                // edit place or drop the dedicated group's toggle (and
                // cancels an outer one when needed).
                let current = if component["enabled"] == true {
                    "on"
                } else {
                    "off"
                };
                let node = node_id.clone();
                body.push(
                    row(
                        "Orientation",
                        self.selector(
                            format!("reverse-{node_id}"),
                            current.to_owned(),
                            vec![
                                ("on".to_owned(), json!(true)),
                                ("off".to_owned(), json!(false)),
                            ],
                            move |enabled| {
                                json!({
                                    "kind": "set_reverse_orientation",
                                    "node_id": node,
                                    "enabled": enabled,
                                })
                            },
                            read_only,
                            feedback,
                        ),
                    )
                    .into_any_element(),
                );
            }
            "object_instance" => {
                if let Some(id) = component["definition"].as_str() {
                    body.push(
                        row("Definition", link(id, components::item_name(items, id)))
                            .into_any_element(),
                    );
                    let definition = id.to_owned();
                    let edit_definition = Button::new(format!("edit-definition-{node_id}"))
                        .label("Edit Definition")
                        .xsmall()
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(RevealDefinition {
                                    id: definition.clone(),
                                }),
                                cx,
                            );
                        });
                    let mut actions = div().flex().gap_2().child(edit_definition);
                    if !read_only {
                        actions = actions.child(
                            Button::new(format!("unpack-instance-{node_id}"))
                                .label("Unpack")
                                .xsmall()
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(UnpackObjectInstance), cx);
                                }),
                        );
                    }
                    body.push(actions.into_any_element());
                }
                let node = node_id.clone();
                body.push(
                    row(
                        "Instance of",
                        self.selector(
                            format!("definition-{node_id}"),
                            components::subtitle(component, items)
                                .trim_start_matches("→ ")
                                .to_owned(),
                            components::definition_choices(items),
                            move |definition| components::definition_edit(&node, definition),
                            read_only,
                            feedback,
                        ),
                    )
                    .into_any_element(),
                );
            }
            "color_space" => body.push(
                row(
                    "Name",
                    div().child(component["value"].as_str().unwrap_or_default().to_owned()),
                )
                .into_any_element(),
            ),
            "options" => {
                for option in component["values"].as_array().into_iter().flatten() {
                    body.push(
                        row(
                            option["name"].as_str().unwrap_or_default().to_owned(),
                            div().child(option["value"].to_string()),
                        )
                        .into_any_element(),
                    );
                }
            }
            _ => {}
        }
        body
    }

    fn header(
        &self,
        icon: assets::IconName,
        name: String,
        kind: String,
        details: HeaderDetails,
    ) -> AnyElement {
        let menu_kind = details.node_kind.clone();
        let menu_read_only = details.read_only;
        let title: AnyElement = match (&self.label_input, details.read_only) {
            (Some((_, _, input)), false) => Input::new(input).small().w_full().into_any_element(),
            _ => div()
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .truncate()
                .child(name)
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pb_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(icon).size(px(18.)))
                    .children(details.active_toggle.map(|(node_id, active)| {
                        let session = self.session.clone();
                        Checkbox::new(SharedString::from(format!("active-{node_id}")))
                            .checked(active)
                            .on_click(move |value: &bool, _, cx| {
                                apply(
                                    &session,
                                    "set-node-active",
                                    active_edit(&node_id, *value),
                                    cx,
                                );
                            })
                    }))
                    .child(div().flex_1().min_w_0().child(title))
                    .child(div().text_color(muted()).child(kind))
                    .child(
                        Button::new("open-in-editor")
                            .icon(assets::IconName::SquarePen)
                            .xsmall()
                            .tooltip(if details.open_in_editor.is_some() {
                                "Open in Editor"
                            } else {
                                "No editor for this kind yet"
                            })
                            .disabled(details.open_in_editor.is_none())
                            .on_click(move |_, window, cx| {
                                if let Some(id) = details.open_in_editor.clone() {
                                    window.dispatch_action(Box::new(OpenInEditor { id }), cx)
                                }
                            }),
                    )
                    .when_some(menu_kind.clone(), |row, kind| {
                        row.child(
                            Button::new("inspector-node-menu")
                                .icon(assets::IconName::EllipsisVertical)
                                .xsmall()
                                .accessibility_label("Node actions")
                                .dropdown_menu(move |menu, _, _| {
                                    inspector_node_menu(menu, &kind, menu_read_only)
                                }),
                        )
                    }),
            )
            .children(
                details
                    .lines
                    .into_iter()
                    .map(|line| div().text_color(muted()).truncate().child(line)),
            )
            .context_menu(move |menu, _, _| {
                if let Some(kind) = menu_kind.as_ref() {
                    inspector_node_menu(menu, kind, menu_read_only)
                } else {
                    menu
                }
            })
            .into_any_element()
    }

    pub(super) fn project_inspector(
        &self,
        details: &Value,
        read_only: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(message) = details["message"].as_str() {
            return div()
                .text_color(muted())
                .child(message.to_owned())
                .into_any_element();
        }
        let mut content = div().flex().flex_col().gap_2();
        if read_only {
            content = content.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(rgb(0x3a3350))
                    .child(div().child("Viewer — save as a project to edit"))
                    .child(
                        Button::new("inspector-save-as-project")
                            .label("Save as Project…")
                            .xsmall()
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(SaveProject), cx)
                            }),
                    ),
            );
        }
        let folded = self.session.read(cx).folded.clone();
        let label_message = self
            .session
            .read(cx)
            .edit_feedback
            .message(&super::label_form_id(details))
            .map(str::to_owned);
        if details["node"].is_object() {
            let node = &details["node"];
            let object = node["data"]["kind"].as_str().unwrap_or("node");
            let kind = match object {
                "entity" => node["data"]["entity"].as_str().unwrap_or("entity"),
                "scope" => "group",
                other => other,
            };
            let mut lines = vec![format!("uuid {}", node["node_id"].as_str().unwrap_or(""))];
            if let Some(line) = node["origin"]["span"]["line"].as_u64() {
                lines.push(format!("source line {line}"));
            }
            let source = lines.join(" · ");
            content = content.child(
                self.header(
                    crate::editor::hierarchy::object_icon_for(object, kind, &node["components"]),
                    node["label"].as_str().unwrap_or("Node").to_owned(),
                    kind.to_owned(),
                    HeaderDetails {
                        lines,
                        open_in_editor: None,
                        read_only,
                        active_toggle: (!read_only
                            && matches!(kind, "shape" | "light" | "instance" | "group"))
                        .then(|| {
                            (
                                node["node_id"].as_str().unwrap_or_default().to_owned(),
                                node["active"] != false,
                            )
                        }),
                        node_kind: Some(kind.to_owned()),
                    },
                ),
            );
            if let Some(message) = label_message.clone() {
                content = content.child(feedback_line(message));
            }
            if let Some(error) = node["components_error"].as_str() {
                content = content.child(div().text_color(rgb(0xd99a74)).child(error.to_owned()));
            }
            for component in node["components"].as_array().cloned().unwrap_or_default() {
                let feedback = self.session.read(cx).edit_feedback.clone();
                let schema = self.session.read(cx).resource_schema.clone();
                let body =
                    self.component_body(&component, details, &schema, read_only, &feedback, cx);
                let remove = component_menu::remove_edits(
                    &component,
                    node["node_id"].as_str().unwrap_or_default(),
                )
                .filter(|_| !read_only);
                // The node's transform form, when the Inspector built one
                // This is the transform form that the ⋮ menu's Apply Transform sends.
                let node_id = node["node_id"].as_str().unwrap_or_default();
                let transform = transform_header_control(
                    &component,
                    self.session.read(cx).transform_matrix,
                    self.transform_forms
                        .iter()
                        .find(|(_, id, _)| id == node_id)
                        .map(|(_, _, form)| form.clone()),
                );
                let camera_align = if component["kind"] == "camera" && !read_only {
                    let session = self.session.read(cx);
                    let coordinate_system: Option<project::CoordinateSystem> = serde_json::from_value(json!({
                        "up": session.project.active_up.unwrap_or(protocol::project::UpAxis::PosY),
                        "handedness": session.project.active_handedness.unwrap_or(protocol::project::Handedness::Left),
                    })).ok();
                    coordinate_system
                        .and(session.view_camera.as_ref())
                        .filter(|_| session.project.active_scene.is_some())
                        .map(|_| json!({"camera_id":node_id}))
                } else {
                    None
                };
                content = content.child(self.component_box(
                    &component,
                    body,
                    format!("{} — {source}", node["label"].as_str().unwrap_or("Node")),
                    &folded,
                    &details["items"],
                    BoxChrome {
                        remove,
                        transform,
                        camera_align,
                    },
                ));
            }
            if !read_only {
                let session = self.session.clone();
                let rows = component_menu::addable(node);
                content = content.child(
                    Button::new("inspector-add-component")
                        .icon(assets::IconName::Plus)
                        .label("Add Component")
                        .xsmall()
                        .dropdown_menu(move |menu, _, _| {
                            let mut menu = menu;
                            if rows.is_empty() {
                                menu = menu.item(
                                    PopupMenuItem::new("No components to add").disabled(true),
                                );
                            }
                            for (label, edit) in rows.clone() {
                                let session = session.clone();
                                menu = menu.item(PopupMenuItem::new(label).on_click(
                                    move |_, _, cx| {
                                        apply(&session, "inspector-add", edit.clone(), cx)
                                    },
                                ));
                            }
                            menu
                        }),
                );
            }
        } else if details["item"].is_object() {
            content = content.child(self.item_inspector(details, read_only, &folded, cx));
            if let Some(message) = label_message {
                content = content.child(feedback_line(message));
            }
        }
        if let Some(error) = details["error"]
            .as_str()
            .or(details["parameters"]["error"].as_str())
        {
            content = content.child(div().text_color(rgb(0xd99a74)).child(error.to_owned()));
        }
        content.into_any_element()
    }

    /// Build the "Used by (N) ▸" list. Each row names
    /// its source and its via, opening with a click — abstracting over
    /// the three reference kinds. Until the backend's answer lands, the
    /// old note stands in.
    fn used_by_row(
        &self,
        kind: &str,
        item: &Value,
        uses: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = item["id"].as_str().unwrap_or_default().to_owned();
        let list = self
            .session
            .update(cx, |session, cx| session.item_references(kind, &id, cx))
            .filter(|list| list.get("references").is_some());
        let Some(list) = list else {
            return div()
                .text_color(rgb(0xd0a86a))
                .child(format!(
                    "Used in {uses} places — edits apply to all of them"
                ))
                .into_any_element();
        };
        let total = list["total"].as_u64().unwrap_or(uses);
        let rows = list["references"].as_array().cloned().unwrap_or_default();
        Button::new("inspector-used-by")
            .label(format!("Used by ({total}) ▸"))
            .xsmall()
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;
                let mut shown = 0usize;
                for row in rows.iter() {
                    let label = reference_label(row);
                    let reveal_row = row.clone();
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        window.dispatch_action(reference_reveal(&reveal_row), cx)
                    }));
                    shown += 1;
                }
                if shown < total as usize {
                    menu = menu.separator().item(PopupMenuItem::new(format!(
                        "and {} more",
                        total - shown as u64
                    )));
                }
                menu
            })
            .into_any_element()
    }

    fn item_inspector(
        &self,
        details: &Value,
        read_only: bool,
        folded: &Folded,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let item = &details["item"];
        let kind = item["kind"].as_str().unwrap_or("item");
        let resource_kind = item["resource_kind"].as_str();
        let mut lines = Vec::new();
        if let Some(id) = item["id"].as_str() {
            lines.push(format!("uuid {id}"));
        }
        if let Some(location) = item["location"].as_str().filter(|l| *l != "@builtin") {
            lines.push(format!("Assets/{location}"));
        }
        let label_kind = resource_kind.unwrap_or(kind).replace('_', " ");
        let icon = match (kind, resource_kind) {
            ("scene", _) => assets::IconName::Clapperboard,
            (_, Some("material")) => assets::IconName::Palette,
            (_, Some("texture")) => assets::IconName::Image,
            (_, Some("medium")) => assets::IconName::Droplets,
            (_, Some("spectrum_data")) => assets::IconName::ChartSpline,
            (_, Some("object_definition")) => assets::IconName::Boxes,
            (_, Some("preset")) => assets::IconName::SquarePlus,
            _ => assets::IconName::FileBox,
        };
        let mut content = div().flex().flex_col().gap_2().child(
            self.header(
                icon,
                item["name"].as_str().unwrap_or_default().to_owned(),
                label_kind,
                HeaderDetails {
                    lines,
                    open_in_editor: editor_panels::editor_for(item)
                        .and(item["id"].as_str())
                        .map(str::to_owned),
                    read_only: read_only || details["resource"].is_null(),
                    active_toggle: None,
                    node_kind: None,
                },
            ),
        );
        let uses = item["reference_count"].as_u64().unwrap_or(0);
        if uses >= 2 && details["resource"].is_object() {
            content = content.child(self.used_by_row(kind, item, uses, cx));
        }
        let resource = &details["resource"];
        if resource.is_object() {
            let target = resource["resource_id"].as_str().unwrap_or_default();
            let mut body = Vec::new();
            let component = json!({
                "kind": resource["kind"],
                "implementation": resource["implementation"],
                "texture_value_type": resource["texture_value_type"],
            });
            let feedback = self.session.read(cx).edit_feedback.clone();
            let schema = self.session.read(cx).resource_schema.clone();
            body.extend(parameter_field::texture_value_type_selector(
                &self.session,
                &component,
                &json!({"kind":"resource","id":target}),
                read_only,
                &feedback,
            ));
            body.extend(self.implementation_row(
                &component,
                &json!({"kind": "resource", "id": target}),
                &schema,
                read_only,
                &feedback,
            ));
            // A texture preview or material sphere is
            // evaluated into core's cache and decoded to a 128px image. The
            // sphere is an approximation, not a PBRT render, and
            // says so.
            let material = resource["kind"] == "material";
            let preview_kind = match resource["kind"].as_str() {
                Some("material") => Some(PreviewKind::Material),
                Some("texture") => Some(PreviewKind::Texture),
                _ => None,
            };
            if let Some(kind) = preview_kind
                && let Some(image) = self
                    .session
                    .update(cx, |session, cx| session.preview(target, kind, cx))
            {
                let image = img(image).size(px(128.)).rounded_sm();
                body.push(
                    row(
                        "Preview",
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(image)
                            .when(material, |column| {
                                column.child(div().text_xs().text_color(rgb(0x9098a6)).child(
                                    preview_caption(
                                        resource["implementation"].as_str().unwrap_or_default(),
                                    ),
                                ))
                            }),
                    )
                    .into_any_element(),
                );
            }
            // A texture's 2D mapping rows sit in their own Mapping box, and
            // a row r4 does not read under the current values is hidden.
            let rows = parameter_rows::rows(&details["parameters"]);
            let shows = |name: &str, mapping: bool| {
                rows.iter().any(|row| {
                    row.name == name
                        && (row.group.as_deref() == Some("mapping")) == mapping
                        && parameter_rows::in_use(&rows, row)
                })
            };
            if resource["kind"] == "object_definition" {
                body.push(row("Instances", div().child(uses.to_string())).into_any_element());
            } else if !item_builds_parameter_rows(item) {
                // A spectrum data resource shows its
                // read-only summary here — its wavelengths/values never
                // become parameter rows, not even read-only ones.
                body.extend(self.spectrum_data_summary_body(target, item, cx));
            } else {
                body.extend(self.parameters(
                    target,
                    &details["parameters"],
                    read_only,
                    &|name| shows(name, false),
                    cx,
                ));
            }
            let label = resource["label"].as_str().unwrap_or("");
            let boxed = self.component_box(
                &component,
                body,
                format!("{label} — uuid {target}"),
                folded,
                &details["items"],
                BoxChrome {
                    remove: None,
                    transform: None,
                    camera_align: None,
                },
            );
            content = content.child(boxed);
            let mapping = rows.iter().any(|row| {
                row.group.as_deref() == Some("mapping") && parameter_rows::in_use(&rows, row)
            });
            if mapping {
                let mode = rows
                    .iter()
                    .find(|row| row.name == "mapping")
                    .and_then(|row| match &row.field {
                        parameter_rows::Field::Text(mode) => Some(mode.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let body = self.parameters(
                    target,
                    &details["parameters"],
                    read_only,
                    &|name| shows(name, true),
                    cx,
                );
                content = content.child(self.component_box(
                    &json!({"kind": "mapping", "implementation": mode}),
                    body,
                    format!("{label} mapping — uuid {target}"),
                    folded,
                    &details["items"],
                    BoxChrome {
                        remove: None,
                        transform: None,
                        camera_align: None,
                    },
                ));
            }
        }
        match kind {
            "scene" => {
                let scene = self.session.read(cx).project_scenes["scenes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|scene| scene["id"] == item["id"])
                    .cloned()
                    .unwrap_or_default();
                content = content
                    .child(row(
                        "Nodes",
                        div().child(format!(
                            "{} directive nodes",
                            scene["node_count"].as_u64().unwrap_or(0)
                        )),
                    ))
                    .child(row(
                        "Default",
                        div().child(if scene["default"] == true {
                            "yes"
                        } else {
                            "no"
                        }),
                    ))
                    .child(row(
                        "Open",
                        div().child(if scene["active"] == true { "yes" } else { "no" }),
                    ))
                    .when(scene["active"] != true, |this| {
                        this.child(
                            Button::new("inspector-open-scene")
                                .label("Open Scene")
                                .xsmall()
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ContentsOpen), cx)
                                }),
                        )
                    });
            }
            "asset" => {
                let asset = &details["asset"];
                // An image file previews at 128px like a texture,
                // ahead of its Media type row; PLY and other non-image
                // assets keep their icon-only rows.
                if PreviewKind::of_entry(item) == Some(PreviewKind::ImageAsset)
                    && let Some(id) = item["id"].as_str()
                    && let Some(image) = self.session.update(cx, |session, cx| {
                        session.preview(id, PreviewKind::ImageAsset, cx)
                    })
                {
                    content = content.child(
                        row(
                            "Preview",
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(img(image).size(px(128.)).rounded_sm()),
                        )
                        .into_any_element(),
                    );
                }
                content = content
                    .child(row(
                        "Media type",
                        div().child(item["media_type"].as_str().unwrap_or("unknown").to_owned()),
                    ))
                    .child(row("Used by", div().child(uses.to_string())))
                    .when_some(asset["size"].as_str(), |this, size| {
                        this.child(row("Size", div().child(components::size_text(size))))
                    })
                    // Show a PLY mesh's header statistics.
                    .when(asset["mesh"].is_object(), |this| {
                        let mesh = &asset["mesh"];
                        let mut text =
                            format!("{} vertices · {} faces", mesh["vertices"], mesh["faces"]);
                        if mesh["normals"] == true {
                            text.push_str(" · normals");
                        }
                        if mesh["uvs"] == true {
                            text.push_str(" · UVs");
                        }
                        this.child(row("Mesh", div().child(text)))
                    })
                    .when_some(asset["sha256"].as_str(), |this, sha| {
                        this.child(row("SHA-256", div().truncate().child(sha.to_owned())))
                    })
                    .when_some(asset["origin"]["logical_path"].as_str(), |this, path| {
                        this.child(row(
                            "Imported from",
                            div().truncate().child(path.to_owned()),
                        ))
                    });
            }
            "unregistered" => {
                content = content.child(
                    div()
                        .text_color(muted())
                        .child("Not registered yet — Save registers it with a .meta file."),
                );
            }
            _ if item["builtin"] == true => {
                let preset = item["id"]
                    .as_str()
                    .and_then(|id| uuid::Uuid::parse_str(id).ok());
                content = content
                    .child(
                        div()
                            .text_color(muted())
                            .child("Built-in preset (read-only)"),
                    )
                    .when_some(preset.filter(|_| !read_only), |this, preset| {
                        this.child(
                            Button::new("inspector-create-preset")
                                .label("Create in Hierarchy")
                                .xsmall()
                                .on_click(move |_, window, cx| {
                                    window.dispatch_action(Box::new(CreatePreset { preset }), cx)
                                }),
                        )
                    });
            }
            _ => {}
        }
        content.into_any_element()
    }
}

fn active_edit(node_id: &str, active: bool) -> Value {
    json!({"kind":"set_node_active","node_id":node_id,"active":active})
}

#[cfg(test)]
mod active_edit_tests {
    use super::active_edit;

    #[test]
    fn active_edit_matches_project_edit_wire_shape() {
        let edit = active_edit("6f0d8a4e-1111-4222-8333-444455556666", false);
        serde_json::from_value::<protocol::project::ProjectEdit>(edit)
            .expect("active edit must match ProjectEdit");
    }
}

/// The caption under a material's sphere preview: approximate always,
/// and — for the metals whose specular depends on reflections — honest
/// about Lit carrying no such data: say so instead of
/// adding reflection shading).
fn preview_caption(implementation: &str) -> &'static str {
    match implementation {
        "conductor" | "coatedconductor" => "Approximate, not a PBRT render · No reflections in Lit",
        _ => "Approximate, not a PBRT render",
    }
}

#[cfg(test)]
mod caption_tests {
    use super::preview_caption;

    /// A conductor or coated conductor's preview caption adds the
    /// Lit caveat to the approximation note; the other materials keep
    /// today's caption.
    #[test]
    fn the_metals_preview_captions_say_lit_has_no_reflections() {
        assert_eq!(
            preview_caption("conductor"),
            "Approximate, not a PBRT render · No reflections in Lit"
        );
        assert_eq!(
            preview_caption("coatedconductor"),
            "Approximate, not a PBRT render · No reflections in Lit"
        );
        assert_eq!(preview_caption("diffuse"), "Approximate, not a PBRT render");
        assert_eq!(
            preview_caption("dielectric"),
            "Approximate, not a PBRT render"
        );
        assert_eq!(preview_caption("mix"), "Approximate, not a PBRT render");
    }
}

#[cfg(test)]
mod reference_tests {
    use super::{reference_label, reference_reveal};
    use crate::editor::menu::{RevealItem, RevealNode};
    use serde_json::json;

    #[test]
    fn labels_name_the_source_and_the_via() {
        // A scene node names its scene and itself.
        let row = json!({
            "kind":"node",
            "scene":{"id":"s","label":"Main"},
            "node":{"id":"n","label":"Floor","object":"group","path":["p"]},
            "via":"material",
        });
        assert_eq!(reference_label(&row), "Main / Floor (material)");
        // A definition's inner node names its owning definition.
        let row = json!({
            "kind":"definition_node",
            "definition":{"id":"d","label":"Tree"},
            "node":{"id":"n","label":"Trunk","object":"shape"},
            "via":"medium inside",
        });
        assert_eq!(reference_label(&row), "Tree / Trunk (medium inside)");
        // A resource names itself, through its parameter.
        let row = json!({
            "kind":"resource",
            "resource":{"id":"r","label":"Alloy","resource_kind":"material"},
            "via":"parameter:amount",
        });
        assert_eq!(reference_label(&row), "Alloy (parameter:amount)");
    }

    #[test]
    fn reveals_point_at_the_right_target() {
        // A scene node reveals itself in the Hierarchy, carrying its
        // ancestor path so the lazy tree can expand to it.
        let row = json!({
            "kind":"node",
            "scene":{"id":"scene-2","label":"Other"},
            "node":{"id":"n","label":"Floor","object":"group",
                    "path":["parent","grandparent"]},
            "via":"material",
        });
        let reveal = reference_reveal(&row);
        assert!(
            reveal.partial_eq(&RevealNode {
                scene: "scene-2".into(),
                node_id: "n".into(),
                path: vec!["parent".into(), "grandparent".into()],
            }),
            "a node row reveals the node in its scene"
        );
        // A definition's inner node and a resource reveal their item in
        // Contents instead - the Hierarchy only shows scene trees.
        let row = json!({
            "kind":"definition_node",
            "definition":{"id":"definition-id","label":"Tree"},
            "node":{"id":"n","label":"Trunk","object":"shape"},
            "via":"medium inside",
        });
        let reveal = reference_reveal(&row);
        assert!(
            reveal.partial_eq(&RevealItem {
                id: "definition-id".into()
            }),
            "a definition row reveals the definition's resource"
        );
        let row = json!({
            "kind":"resource",
            "resource":{"id":"resource-id","label":"Alloy","resource_kind":"material"},
            "via":"parameter:amount",
        });
        let reveal = reference_reveal(&row);
        assert!(
            reveal.partial_eq(&RevealItem {
                id: "resource-id".into()
            }),
            "a resource row reveals the resource"
        );
    }
}

#[cfg(test)]
mod spectrum_summary_tests {
    use super::{item_builds_parameter_rows, spectrum_samples_text};
    use serde_json::json;

    /// Check that the Samples summary line spells the sample
    /// count, the first-to-last wavelength range and the highest value
    /// the way the Spectrum Data editor renders cells (`cell_number`),
    /// unpadded — a one-sample curve's range does not widen the way
    /// the plot's axes would.
    #[test]
    fn sample_lines_spell_count_range_and_peak_like_the_editor_cells() {
        let table = json!({"wavelengths":[380., 550.5, 780.],
                           "values":[0.2, 1.2, 0.8], "storage":"floats"});
        assert_eq!(
            spectrum_samples_text(Some(&table)),
            "3 samples · 380 – 780 nm · values up to 1.2"
        );
        let single = json!({"wavelengths":[600.],"values":[1.],"storage":"floats"});
        assert_eq!(
            spectrum_samples_text(Some(&single)),
            "1 sample · 600 – 600 nm · values up to 1"
        );
    }

    /// Check that a pending table shows the ellipsis, an
    /// `{error}` payload its message, an empty table its bare count.
    #[test]
    fn pending_tables_show_an_ellipsis_and_errors_their_message() {
        assert_eq!(spectrum_samples_text(None), "…");
        assert_eq!(
            spectrum_samples_text(Some(&json!({"error":"no such resource"}))),
            "no such resource"
        );
        assert_eq!(
            spectrum_samples_text(Some(&json!({"wavelengths":[],"values":[]}))),
            "0 samples"
        );
    }

    /// Check that a spectrum data resource's page builds no
    /// parameter rows for its wavelengths/values — the read-only
    /// summary replaces them — while every other item keeps its rows.
    #[test]
    fn spectrum_data_pages_build_no_parameter_rows() {
        let spectrum = json!({"kind":"resource","resource_kind":"spectrum_data"});
        assert!(!item_builds_parameter_rows(&spectrum));
        for kind in ["material", "texture", "medium", "object_definition"] {
            assert!(
                item_builds_parameter_rows(&json!({"kind":"resource","resource_kind":kind})),
                "{kind} keeps its parameter rows"
            );
        }
        // Node pages carry no resource kind: their rows keep building.
        assert!(item_builds_parameter_rows(&json!({"kind":"node"})));
        assert!(item_builds_parameter_rows(&serde_json::Value::Null));
    }
}

#[cfg(test)]
mod transform_header_tests {
    use super::{TransformHeaderControl, local_has_trs, transform_header_control};
    use serde_json::json;

    /// The header's TRS button is disabled exactly when the local
    /// has no TRS form — a single local needs its own `trs`, an animated
    /// one a `trs` for every endpoint that carries a matrix.
    #[test]
    fn trs_availability_covers_single_and_animated_locals() {
        assert!(local_has_trs(
            &json!({"trs": {"translation": [0., 0., 0.]}})
        ));
        assert!(!local_has_trs(&json!({})));
        // Animated: every endpoint with a matrix must offer its TRS.
        let animated = json!({
            "samples": {
                "start": {"matrix": [0., 0.], "trs": {"translation": [0., 0., 0.]}},
                "end":   {"matrix": [0., 0.], "trs": {"translation": [1., 0., 0.]}}
            }
        });
        assert!(local_has_trs(&animated));
        let one_broken = json!({
            "samples": {
                "start": {"matrix": [0., 0.], "trs": {"translation": [0., 0., 0.]}},
                "end":   {"matrix": [0., 0.]}
            }
        });
        assert!(!local_has_trs(&one_broken));
        let no_matrices = json!({
            "samples": {"start": {}, "end": {}}
        });
        assert!(!local_has_trs(&no_matrices));
    }

    /// The header switch follows the session's chosen display while
    /// TRS exists, falls back to the matrix when it does not, and shows
    /// nothing where the previous Mode row showed nothing. The form
    /// rides along for the ⋮ menu's Apply Transform.
    #[test]
    fn the_header_switch_follows_availability_and_the_session_choice() {
        let trs_and_matrix = json!({
            "kind": "transform",
            "local": {"matrix": [16], "trs": {"translation": [0., 0., 0.]}}
        });
        assert_eq!(
            transform_header_control(&trs_and_matrix, false, None),
            Some(TransformHeaderControl {
                matrix: false,
                has_trs: true,
                form: None
            })
        );
        assert_eq!(
            transform_header_control(&trs_and_matrix, true, None),
            Some(TransformHeaderControl {
                matrix: true,
                has_trs: true,
                form: None
            })
        );
        // No TRS: the display stays on the matrix whatever the session says.
        let matrix_only = json!({
            "kind": "transform",
            "local": {"matrix": [16]}
        });
        assert_eq!(
            transform_header_control(&matrix_only, false, None),
            Some(TransformHeaderControl {
                matrix: true,
                has_trs: false,
                form: None
            })
        );
        // An undisplayable local, and other components: no switch.
        assert_eq!(
            transform_header_control(&json!({"kind": "transform", "local": {}}), false, None),
            None
        );
        assert_eq!(
            transform_header_control(&json!({"kind": "shape"}), false, None),
            None
        );
    }
}
