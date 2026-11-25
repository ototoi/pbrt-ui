//! The Material / Texture Editor: the opened resource's upstream graph
//! from `project.get_resource_graph`, laid out by `graph_layout` — the
//! graph view and its operations only (the: the
//! parameters live in the Inspector, which every selection drives).
//! Left-drag the background to pan, wheel to zoom; Frame All fits the
//! graph. Drag input slots onto resource nodes or resource outputs onto
//! compatible input rows to connect them;
//! spectrum values ride as value nodes, replaceable through their menu.
//! Right-click the background to create a Material or
//! Texture there — `node_editor_create` holds its pure decisions.
use super::Session;
use super::graph_layout::{self, HEADER, INPUT_ROW, NODE_WIDTH};
use super::node_editor_create::{self, CreateMenu};
use super::parameter_rows;
use crate::backend::{Command, PreviewKind};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable, button::Button};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

pub struct NodeEditor {
    session: Entity<Session>,
    root: String,
    selected: String,
    /// The project reference the graph and details were last asked for.
    requested: Value,
    details_requested: String,
    pan: (f32, f32),
    zoom: f32,
    last_mouse: Option<Point<Pixels>>,
    drag_origin: Option<Point<Pixels>>,
    /// Material / Texture node positions; a folder project saves them (session).
    positions: HashMap<String, (f32, f32)>,
    /// The session's position load this editor last took its layout from.
    positions_loaded: u64,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    /// While a slot is dragged: the line from the slot to the pointer, in
    /// viewport pixels.
    drag_line: Option<((f32, f32), (f32, f32))>,
    /// The node ids the last render drew: a set change —
    /// the first graph included — lets the view fit them when a card
    /// falls off screen; same-set edits never move the view.
    drawn: Option<HashSet<String>>,
    /// A fit the view still owes, waiting for the viewport's size.
    frame_pending: bool,
    /// The ids created from this editor's background menu
    /// whose resources still exist, in creation order — editor
    /// state, never saved, like `positions`: it dies with the editor's
    /// tab while the resource stays in Contents.
    extra: Vec<String>,
    /// The graph-units point of the right-click that opened the create
    /// menu, held from the press until the menu's pick rides it with its
    /// ask.
    pending_create: Option<(f32, f32)>,
}

/// One background creation in flight — the editor that
/// asked, and the graph-units click where its node lands.
pub(super) struct CreateAsk {
    pub(super) editor: WeakEntity<NodeEditor>,
    pub(super) point: Option<(f32, f32)>,
}

/// What an input slot carries while it is dragged onto a graph node.
#[derive(Clone)]
struct SlotDrag {
    target: String,
    parameter: String,
    index: Option<usize>,
    accepts: String,
    materials: Vec<String>,
    previous: Option<String>,
    /// Whether the row allows a SpectrumData node in this slot.
    spectrum_data: bool,
    /// The table marks the slot's operand required: replacements ride
    /// the drag as ever, removals are the menu's business instead.
    required: bool,
    /// The declaration the slot holds as written (name and value
    /// type) — a SpectrumData drop replaces it first, naming its own
    /// spelling.
    declared: Option<(String, String)>,
    /// The slot's point in graph units.
    from: (f32, f32),
}

/// What a resource node carries while its output is dragged to an input row.
#[derive(Clone)]
struct OutputDrag {
    id: String,
    kind: String,
    texture_value_type: String,
    from: (f32, f32),
}

impl OutputDrag {
    fn node(&self) -> Value {
        json!({"id":self.id,"kind":self.kind,"texture_value_type":self.texture_value_type})
    }
}

fn connection_edits(drag: &SlotDrag, node: &Value) -> Vec<Value> {
    let target = json!({"kind":"resource","id":drag.target});
    if drag.accepts == "material" {
        vec![parameter_rows::mix_material_edit(
            &target,
            &drag.materials,
            drag.index.unwrap_or(0),
            node["id"].as_str().unwrap_or_default(),
        )]
    } else if node["kind"] == "spectrum_data" {
        parameter_rows::spectrum_connect_edits(
            &target,
            &drag.parameter,
            drag.declared
                .as_ref()
                .map(|(name, value_type)| (name.as_str(), value_type.as_str())),
            drag.required,
            node["id"].as_str().unwrap_or_default(),
        )
    } else {
        vec![json!({"kind":"connect_texture",
            "target":target,
            "parameter":drag.parameter,
            "texture":node["id"].as_str().unwrap_or_default()})]
    }
}

fn output_can_connect(
    graph: &Value,
    drag: &OutputDrag,
    accepts: &str,
    spectrum_data: bool,
    target: &str,
) -> bool {
    graph_layout::can_connect(graph, &drag.node(), accepts, spectrum_data, target)
}

/// What a header drag is repositioning.
struct NodeDrag {
    id: String,
    x: f32,
    y: f32,
}

/// What one node card draws with: the graph it belongs to (menus read
/// the slot rows of it), whether the document takes edits, the decoded
/// preview a material or texture card shows, and the Inspector-driven
/// highlight.
struct NodeCard {
    graph: Rc<Value>,
    read_only: bool,
    preview: Option<Arc<RenderImage>>,
    highlighted: Option<String>,
}

/// The label that follows the pointer during a slot drag.
struct SlotLabel(String);

impl Render for SlotLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(rgb(0x3a3d45))
            .text_sm()
            .text_color(rgb(0xd4d7dd))
            .child(self.0.clone())
    }
}

impl NodeEditor {
    pub fn new(session: Entity<Session>, root: String, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |this, _, cx| {
            this.sync(cx);
            cx.notify()
        })
        .detach();
        let positions = session.read(cx).node_positions.for_root(&root);
        let positions_loaded = session.read(cx).node_positions_loaded;
        let mut editor = Self {
            session,
            selected: root.clone(),
            root,
            requested: Value::Null,
            details_requested: String::new(),
            pan: (0., 0.),
            zoom: 1.,
            last_mouse: None,
            drag_origin: None,
            positions,
            positions_loaded,
            viewport: Rc::new(Cell::new(Bounds::default())),
            drag_line: None,
            drawn: None,
            frame_pending: false,
            extra: Vec::new(),
            pending_create: None,
        };
        editor.sync(cx);
        editor
    }

    /// Ask again for the graph and the selected node on every revision.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let project = self.session.read(cx).project_value();
        if !project.is_object() {
            return;
        }
        // A tab restored with the layout opens before the project's saved
        // positions are read; take them when they arrive.
        let loaded = self.session.read(cx).node_positions_loaded;
        if loaded != self.positions_loaded {
            self.positions_loaded = loaded;
            self.positions = self.session.read(cx).node_positions.for_root(&self.root);
        }
        // Ids created here stop being extra once their resource is
        // gone.
        self.prune_extras(&project, cx);
        let stale = project != self.requested;
        if stale {
            self.requested = project.clone();
            let root = self.root.clone();
            // Each unconnected created id asks for its graph the same
            // way, at the revision that just landed; an answer read at a
            // project still current stands.
            let asks: Vec<String> = {
                let session = self.session.read(cx);
                self.extra
                    .iter()
                    .filter(|id| {
                        !session
                            .resource_graphs
                            .get(id.as_str())
                            .is_some_and(|(read_at, _)| *read_at == project)
                    })
                    .cloned()
                    .collect()
            };
            self.session.update(cx, |session, cx| {
                session.send(
                    Command::ResourceGraph {
                        project: project.clone(),
                        resource_id: root,
                    },
                    cx,
                );
                for id in asks {
                    session.send(
                        Command::ResourceGraph {
                            project: project.clone(),
                            resource_id: id,
                        },
                        cx,
                    );
                }
            });
        }
        // The selected node's page: a value node is its owner's
        // parameter, so the details ask names the owner — including
        // when the node just vanished from the graph (its own menu or
        // the page's Data tab consumed it): the value page keeps its
        // owner's data across the very edits it sends.
        let (selected, root) = (self.selected.clone(), self.root.clone());
        let extra_ids = self.extra.clone();
        let details_id = self.session.update(cx, |session, _| {
            let graph = session
                .resource_graphs
                .get(&root)
                .filter(|(read_at, _)| *read_at == project)
                .map(|(_, graph)| graph.clone())
                .unwrap_or_default();
            // The ids created unconnected here merge in at this
            // revision, so a creation's selection, its successors and its
            // details read one graph with everything else.
            let extras = extra_ids
                .iter()
                .filter_map(|id| {
                    session
                        .resource_graphs
                        .get(id)
                        .filter(|(read_at, _)| *read_at == project)
                        .map(|(_, graph)| graph.clone())
                })
                .collect::<Vec<_>>();
            let graph = node_editor_create::merged_graph(&graph, &extras);
            let successor =
                super::graph_selection::value_successor(&root, &graph, session.graph_value());
            (
                super::graph_selection::details_target(
                    &root,
                    &selected,
                    &graph,
                    session.graph_value(),
                ),
                successor,
            )
        });
        let (details_id, successor) = details_id;
        // The value node the Inspector showed was replaced: select what
        // took its place, so the Inspector shows that node instead of a
        // parameter that is no longer a spectrum value.
        if let Some(next) = successor {
            let shown = self.session.update(cx, |session, cx| {
                let entry = session.known_item(&next)?;
                let project = session.project_value();
                session.select_contents(entry, project, cx);
                Some(())
            });
            if shown.is_some() {
                self.selected = next;
            }
        }
        if stale || self.details_requested != details_id {
            self.details_requested = details_id.clone();
            self.session.update(cx, |session, cx| {
                session.send(
                    Command::ResourceDetails {
                        project,
                        resource_id: details_id,
                    },
                    cx,
                )
            });
        }
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        if self.selected != id {
            self.selected = id;
            self.sync(cx);
            cx.notify();
        }
    }

    /// Drop extra ids whose latest graph answer — read
    /// at the project still current — says the resource is gone (an undo
    /// of the creation, a delete). A connected id stays extra, so undoing
    /// the connection leaves its node on screen; the merge dedupes it.
    fn prune_extras(&mut self, project: &Value, cx: &Context<Self>) {
        let session = self.session.read(cx);
        self.extra.retain(|id| {
            !session
                .resource_graphs
                .get(id.as_str())
                .is_some_and(|(read_at, answer)| {
                    read_at == project && node_editor_create::resource_is_gone(answer)
                })
        });
    }

    /// One menu row's creation uses the same `create_resource`
    /// edit Contents ▸ New applies, in the kind's default folder
    /// (Materials/ or Textures/), in one undoable apply. This editor
    /// takes the answer: the node lands at the right-click that opened
    /// the menu (the point rides the ask, not the editor, so a second
    /// creation cannot steal the first's), the selection follows it,
    /// and no rename prompt opens.
    fn create_from_menu(
        &mut self,
        kind: &str,
        implementation: &str,
        value_type: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let (editor, point) = (cx.entity().downgrade(), self.pending_create.take());
        self.session.update(cx, |session, cx| {
            let project = session.project_value();
            if !project.is_object() || session.document().is_read_only() {
                return;
            }
            let edit = node_editor_create::create_edit(kind, implementation, value_type);
            let folder = super::resource_menu::default_folder(kind).to_owned();
            let ask = session.await_node_create(editor, point);
            session.send(
                Command::Apply {
                    project,
                    edits: vec![edit],
                    follow: crate::backend::Follow::SelectResourceInEditor { folder, ask },
                    form: None,
                },
                cx,
            );
        });
        cx.notify();
    }

    /// The creation applied and the backend named the
    /// resource: the node joins this editor unconnected, sitting at the
    /// click that asked for it (as the ask carried it), and takes the
    /// selection a card click would; its graph is asked for at the
    /// revision that follows.
    pub(super) fn node_created(
        &mut self,
        id: String,
        point: Option<(f32, f32)>,
        cx: &mut Context<Self>,
    ) {
        if let Some(point) = point {
            self.positions.insert(id.clone(), point);
            let root = self.root.clone();
            let id = id.clone();
            self.session.update(cx, |session, _| {
                session.remember_node_position(&root, &id, point);
            });
        }
        self.remember_extra(&id);
        // The revision holding the id may already be the one `sync` asked
        // at; forget it so this sync asks for the new id's graph too.
        self.requested = Value::Null;
        self.selected = id;
        self.sync(cx);
        cx.notify();
    }

    fn remember_extra(&mut self, id: &str) {
        if !self.extra.iter().any(|extra| extra == id) {
            self.extra.push(id.to_owned());
        }
    }

    /// Fit the laid-out graph into the viewport; false when nothing
    /// was fitted — no measured viewport yet, or an empty graph.
    fn frame_all(&mut self, layout: &graph_layout::Layout) -> bool {
        let bounds = self.viewport.get();
        let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        if width <= 0. || layout.width <= 0. || layout.height <= 0. {
            return false;
        }
        self.zoom = (width / layout.width)
            .min(height / layout.height)
            .clamp(0.25, 1.5);
        self.pan = (
            (width - layout.width * self.zoom) / 2. - layout.min_x * self.zoom,
            (height - layout.height * self.zoom) / 2. - layout.min_y * self.zoom,
        );
        true
    }

    /// Send one `project.apply` — one transaction, one Undo step — when
    /// the document takes it. An empty edits list (a menu whose section
    /// had nothing to give) stays silent.
    fn apply_edits(&mut self, edits: Vec<Value>, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            if edits.is_empty() {
                return;
            }
            let project = session.project_value();
            if project.is_object() && !session.document().is_read_only() {
                session.send(
                    Command::Apply {
                        project,
                        edits,
                        follow: crate::backend::Follow::Nothing,
                        form: None,
                    },
                    cx,
                );
            }
        });
        cx.notify();
    }

    /// A slot dropped onto a graph node: a texture takes one
    /// `connect_texture` edit; a SpectrumData takes the colour
    /// dialog's declaration edits — a cross-type declaration is removed
    /// first, and both edits ride the one transaction, so one Undo step
    /// returns the slot's old value. `connect_texture` never applies to a
    /// SpectrumData: core would refuse it for not being a texture.
    fn connect(&mut self, drag: &SlotDrag, node: &Value, cx: &mut Context<Self>) {
        self.drag_line = None;
        let edits = connection_edits(drag, node);
        if let Some(displaced) =
            node_editor_create::displaced_resource(drag.previous.as_deref(), node["id"].as_str())
        {
            self.remember_extra(&displaced);
        }
        self.apply_edits(edits, cx);
    }

    fn output_node_box(
        &self,
        layout: &graph_layout::NodeBox,
        node: &Value,
        zoom: f32,
    ) -> AnyElement {
        let id = node["id"].as_str().unwrap_or_default().to_owned();
        let drag = OutputDrag {
            id,
            kind: node["kind"].as_str().unwrap_or_default().to_owned(),
            texture_value_type: node["texture_value_type"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            from: layout.output(),
        };
        div()
            .id(SharedString::from(format!("graph-output-{}", drag.id)))
            .absolute()
            .right(px(-4. * zoom))
            .top(px((HEADER / 2. - 4.) * zoom))
            .size(px(8. * zoom))
            .rounded_full()
            .bg(rgb(0x7fb2f0))
            .border_1()
            .border_color(rgb(0x2a2d34))
            .cursor_grab()
            .on_drag(drag, move |_, _, _, cx| {
                cx.new(|_| SlotLabel("Connect output".to_owned()))
            })
            .into_any_element()
    }

    fn connect_output(&mut self, drag: &OutputDrag, slot: &SlotDrag, cx: &mut Context<Self>) {
        self.connect(slot, &drag.node(), cx);
    }

    fn node_box(
        &self,
        node: &Value,
        layout: &graph_layout::NodeBox,
        graph: &Value,
        card: NodeCard,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // A value node is its own card: the approximate colour its value
        // shows and the kind it was given, with the replacement menu —
        // no curve sample, no slot rows.
        if node["kind"] == "spectrum_value" {
            return self.value_node_box(node, layout, card, cx);
        }
        let zoom = self.zoom;
        let selected = card.highlighted.as_deref() == Some(layout.id.as_str());
        let id = layout.id.clone();
        let (drop_node, drop_graph, connect_node) = (node.clone(), graph.clone(), node.clone());
        let clicked = node.clone();
        let menu_id = id.clone();
        let read_only = card.read_only;
        let disconnects = parameter_rows::node_disconnect_menu(graph, &id, read_only);
        let delete_entry = parameter_rows::node_delete_allowed(&id, &self.root, read_only)
            .then(|| self.session.read(cx).known_item(&id))
            .flatten()
            .filter(super::contents_tree::editable);
        let editor = cx.entity().downgrade();
        let spectrum_data = node["kind"] == "spectrum_data";
        let (title, icon) = match node["kind"].as_str() {
            Some("texture") => ("texture", assets::IconName::Image),
            Some("spectrum_data") => ("spectrum data", assets::IconName::ChartSpline),
            _ => ("material", assets::IconName::Palette),
        };
        let subtitle = match node["texture_value_type"].as_str() {
            Some(value_type) => format!(
                "{} · {value_type} {title}",
                node["implementation"].as_str().unwrap_or_default()
            ),
            None => format!(
                "{} · {title}",
                node["implementation"].as_str().unwrap_or_default()
            ),
        };
        // A SpectrumData node carries one small approximate colour
        // swatch where a material's sphere or a texture's preview sits —
        // the wire's project.spectrum_color through the same
        // per-revision cache the colour box uses, off the UI thread.
        // Every slot a material / texture graph joins reads its curve
        // the albedo way, so the ask is the albedo read.
        let swatch = spectrum_data.then(|| self.spectrum_swatch(&id, cx));
        // Only slots connect_texture accepts drag.
        let connectable = |parameter: &str, index: Option<usize>| {
            node["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|input| {
                    input["parameter"] == parameter && input["connectable"] == true
                        || input["parameter"] == parameter
                            && input["accepts"] == "material"
                            && index.is_some()
                })
        };
        let accepts = |parameter: &str| {
            node["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|input| input["parameter"] == parameter)
                .and_then(|input| input["accepts"].as_str())
                .unwrap_or("")
                .to_owned()
        };
        // The row's own flag indicates whether a SpectrumData node may be
        // dropped into the slot — and the declaration it holds as
        // written, which the drop replaces first.
        let takes_spectrum_data = |parameter: &str| {
            node["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|input| input["parameter"] == parameter && input["spectrum_data"] == true)
        };
        let declared = |parameter: &str| {
            node["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|input| input["parameter"] == parameter)
                .and_then(|input| {
                    Some((
                        input["declared_name"].as_str()?.to_owned(),
                        input["declared_type"].as_str()?.to_owned(),
                    ))
                })
        };
        div()
            .id(SharedString::from(format!("graph-node-{id}")))
            .absolute()
            .left(px(layout.x * zoom + self.pan.0))
            .top(px(layout.y * zoom + self.pan.1))
            .w(px(NODE_WIDTH * zoom))
            .h(px(layout.height * zoom))
            .text_size(px(12. * zoom))
            .bg(rgb(0x2a2d34))
            .border_1()
            .border_color(if selected {
                rgb(0x7fb2f0)
            } else {
                rgb(0x41444d)
            })
            .rounded_md()
            .cursor_pointer()
            .child(
                div()
                    .id(SharedString::from(format!("graph-node-header-{id}")))
                    .h(px(HEADER * zoom))
                    .px(px(6. * zoom))
                    .flex()
                    .items_center()
                    .gap(px(4. * zoom))
                    .bg(rgb(0x33363e))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.drag_origin = Some(event.position);
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(
                                div()
                                    .flex()
                                    .gap(px(4. * zoom))
                                    .items_center()
                                    .child(Icon::new(icon).size(px(12. * zoom)))
                                    .child(div().truncate().font_weight(FontWeight::BOLD).child(
                                        node["label"].as_str().unwrap_or_default().to_owned(),
                                    )),
                            )
                            .child(div().truncate().text_color(rgb(0x9098a6)).child(subtitle)),
                    )
                    // The texture's preview or the material's sphere, decoded from
                    // core's cache file. A SpectrumData node shows its one small colour
                    // swatch instead — no curve thumbnail.
                    .children(card.preview.clone().map(|image| {
                        img(image)
                            .flex_shrink_0()
                            .size(px((HEADER - 8.) * zoom))
                            .rounded_sm()
                    }))
                    .children(swatch)
                    .children(
                        (!card.read_only
                            && layout.id != self.root
                            && matches!(
                                node["kind"].as_str(),
                                Some("material" | "texture" | "spectrum_data")
                            ))
                        .then(|| self.output_node_box(layout, node, zoom)),
                    )
                    .cursor_grab()
                    .on_drag(
                        NodeDrag {
                            id: layout.id.clone(),
                            x: layout.x,
                            y: layout.y,
                        },
                        move |_, _, _, cx| cx.new(|_| SlotLabel("Move node".to_owned())),
                    ),
            )
            .children(layout.inputs.iter().map(|parameter| {
                let (parameter_name, index) = graph_layout::input_parts(parameter);
                let parameter_name = parameter_name.to_owned();
                let input = node["inputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|input| input["parameter"] == parameter_name)
                    .cloned()
                    .unwrap_or_else(|| json!({"parameter":parameter}));
                let mut slot = div()
                    .id(SharedString::from(format!(
                        "graph-slot-{}-{parameter}",
                        layout.id
                    )))
                    .h(px(INPUT_ROW * zoom))
                    .px(px(8. * zoom))
                    .flex()
                    .items_center()
                    .gap(px(4. * zoom))
                    .child(div().size(px(6. * zoom)).rounded_full().bg(
                        match accepts(&parameter_name).as_str() {
                            "float" => rgb(0x9aa0aa),
                            "material" => rgb(0xd0a86a),
                            _ => rgb(0x7fb2f0),
                        },
                    ))
                    .child(
                        div()
                            .truncate()
                            .text_color(rgb(0xc4c8d0))
                            .child(graph_layout::input_label(parameter)),
                    );
                // A slot without a value in it greys r4's table default
                // beside the name a value node would take.
                if let Some(default) =
                    super::graph_selection::empty_slot_default(graph, &layout.id, &input)
                {
                    slot = slot.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(0x9098a6))
                            .text_right()
                            .child(default),
                    );
                }
                // An empty spectrum-capable slot can grow a value node
                // from its own row: "Place RGB" and kin, the shapes the
                // table allows, and "Place Spectrum Data" listing the
                // project's resources. The probe decides
                // whether the row shows the button at all.
                if !card.read_only
                    && takes_spectrum_data(parameter)
                    && declared(parameter).is_none()
                {
                    let session = self.session.clone();
                    let items = self.session.read(cx).current_items();
                    let target_node = layout.id.clone();
                    let input = input.clone();
                    let builtins = self.session.read(cx).resource_schema["spectra"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|name| name.as_str().map(str::to_owned))
                        .collect::<Vec<_>>();
                    if !parameter_rows::placement_menu(
                        &json!({"kind":"resource","id":layout.id}),
                        &input,
                        &items,
                        &builtins,
                    )
                    .is_empty()
                    {
                        slot = slot.child(div().flex_1().min_w_0());
                        slot = slot.child(
                            Button::new(SharedString::from(format!(
                                "graph-slot-place-{}-{parameter}",
                                layout.id
                            )))
                            .icon(assets::IconName::Plus)
                            .xsmall()
                            .tooltip("Place a value in this slot")
                            .dropdown_menu(
                                move |menu, window, cx| {
                                    let mut menu = menu.scrollable(true).max_h(px(320.));
                                    let entries = parameter_rows::placement_menu(
                                        &json!({"kind":"resource","id":target_node}),
                                        &input,
                                        &items,
                                        &builtins,
                                    );
                                    for (label, edits) in entries.items.clone() {
                                        let (session, edits) = (session.clone(), edits.clone());
                                        menu = menu.item(PopupMenuItem::new(label).on_click(
                                            move |_, _, cx| menu_apply(&session, &edits, cx),
                                        ));
                                    }
                                    // The project's Spectrum Data, behind one
                                    // submenu entry (gpui-kit draws the
                                    // chevron itself): each pick applies the
                                    // edit the Inspector's Data tab makes.
                                    if !entries.spectra.is_empty() {
                                        let session = session.clone();
                                        let spectra = entries.spectra.clone();
                                        menu = menu.submenu(
                                            "Place Spectrum Data",
                                            window,
                                            cx,
                                            move |menu, _, _| {
                                                let mut menu =
                                                    menu.scrollable(true).max_h(px(320.));
                                                for (label, edits) in spectra.iter() {
                                                    let (session, label) =
                                                        (session.clone(), label.clone());
                                                    let edits = edits.clone();
                                                    menu = menu.item(
                                                        PopupMenuItem::new(label).on_click(
                                                            move |_, _, cx| {
                                                                menu_apply(&session, &edits, cx)
                                                            },
                                                        ),
                                                    );
                                                }
                                                menu
                                            },
                                        );
                                    }
                                    menu
                                },
                            ),
                        );
                    }
                }
                let target = json!({"kind":"resource","id":layout.id});
                let connected_edge = graph["edges"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|edge| {
                        edge["to"] == layout.id
                            && edge["parameter"] == parameter_name
                            && (index.is_none()
                                || edge["index"].as_u64().map(|value| value as usize) == index)
                    })
                    .cloned();
                let row_edit = parameter_rows::row_disconnect_menu_edit(
                    &target,
                    &parameter_name,
                    input["required"] == true || accepts(&parameter_name) == "material",
                    connected_edge.is_some(),
                    card.read_only,
                    input["connectable"] == true,
                    input["declared_type"].as_str(),
                );
                let row_menu = row_edit.map(|edit| {
                    (
                        edit,
                        connected_edge.as_ref().and_then(|edge| {
                            node_editor_create::resource_source_for_edge(graph, edge)
                        }),
                        cx.entity().downgrade(),
                    )
                });
                // Slots drag onto texture nodes; a spectrum-capable slot
                // also onto SpectrumData nodes.
                let accepts = accepts(&parameter_name);
                let slot_drag = SlotDrag {
                    target: layout.id.clone(),
                    parameter: parameter_name.clone(),
                    index,
                    accepts: accepts.clone(),
                    materials: {
                        let mut materials = graph["edges"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|edge| {
                                edge["to"] == layout.id && edge["parameter"] == parameter_name
                            })
                            .map(|edge| {
                                (
                                    edge["index"].as_u64().unwrap_or(0) as usize,
                                    edge["from"].as_str().unwrap_or_default().to_owned(),
                                )
                            })
                            .collect::<Vec<_>>();
                        materials.sort_by_key(|(index, _)| *index);
                        materials.into_iter().map(|(_, id)| id).collect()
                    },
                    previous: connected_edge
                        .as_ref()
                        .and_then(|edge| node_editor_create::resource_source_for_edge(graph, edge)),
                    spectrum_data: takes_spectrum_data(parameter),
                    required: input["required"] == true,
                    declared: declared(parameter),
                    from: layout.input(parameter).unwrap_or_default(),
                };
                let mut slot = match layout.input(parameter) {
                    Some(from)
                        if !card.read_only
                            && connectable(&parameter_name, index)
                            && (matches!(accepts.as_str(), "float" | "spectrum")
                                || accepts == "material") =>
                    {
                        let label = graph_layout::input_label(parameter);
                        let required = input["required"] == true;
                        let materials = if accepts == "material" {
                            graph["edges"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|edge| {
                                    edge["to"] == layout.id && edge["parameter"] == parameter_name
                                })
                                .map(|edge| {
                                    (
                                        edge["index"].as_u64().unwrap_or(0) as usize,
                                        edge["from"].as_str().unwrap_or_default().to_owned(),
                                    )
                                })
                                .collect::<HashMap<_, _>>()
                                .into_iter()
                                .collect::<Vec<_>>()
                        } else {
                            Vec::new()
                        };
                        let mut materials = materials;
                        materials.sort_by_key(|(index, _)| *index);
                        let materials = materials.into_iter().map(|(_, id)| id).collect::<Vec<_>>();
                        slot.cursor_grab().on_drag(
                            SlotDrag {
                                target: layout.id.clone(),
                                parameter: parameter_name.clone(),
                                index,
                                accepts: accepts.clone(),
                                materials,
                                previous: connected_edge.as_ref().and_then(|edge| {
                                    node_editor_create::resource_source_for_edge(graph, edge)
                                }),
                                spectrum_data: takes_spectrum_data(parameter),
                                required,
                                declared: declared(parameter),
                                from,
                            },
                            move |_, _, _, cx| cx.new(|_| SlotLabel(label.clone())),
                        )
                    }
                    _ => slot,
                };
                if !card.read_only && connectable(&parameter_name, index) {
                    let drop_graph = graph.clone();
                    let drop_slot = SlotDrag {
                        materials: slot_drag.materials.clone(),
                        ..slot_drag
                    };
                    let can_drop_slot = drop_slot.clone();
                    slot = slot
                        .can_drop(move |value, _, _| {
                            value.downcast_ref::<OutputDrag>().is_some_and(|drag| {
                                output_can_connect(
                                    &drop_graph,
                                    drag,
                                    &can_drop_slot.accepts,
                                    can_drop_slot.spectrum_data,
                                    &can_drop_slot.target,
                                )
                            })
                        })
                        .drag_over::<OutputDrag>(|style, _, _, _| style.bg(rgb(0x3b5142)))
                        .on_drop(cx.listener(move |this, drag: &OutputDrag, _, cx| {
                            this.connect_output(drag, &drop_slot, cx);
                        }));
                }
                if let Some((edit, displaced, editor)) = row_menu {
                    slot.context_menu(move |menu, _, _| {
                        let edit = edit.clone();
                        let displaced = displaced.clone();
                        let editor = editor.clone();
                        menu.item(PopupMenuItem::new("Disconnect").on_click(move |_, _, cx| {
                            let _ = editor.update(cx, |this, cx| {
                                if let Some(extra) = node_editor_create::displaced_resource(
                                    displaced.as_deref(),
                                    None,
                                ) {
                                    this.remember_extra(&extra);
                                }
                                this.apply_edits(vec![edit.clone()], cx);
                            });
                        }))
                    })
                    .into_any_element()
                } else {
                    slot.into_any_element()
                }
            }))
            // Selecting a node drives the Inspector: resource
            // nodes select like a Contents click, a value node opens
            // the inline colour page of its owner's parameter, and a
            // closed Inspector panel opens for either.
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                let double = event.click_count() >= 2;
                this.select(id.clone(), cx);
                window.dispatch_action(Box::new(super::menu::ShowInspector), cx);
                match super::graph_selection::click_target(&clicked) {
                    Some(super::graph_selection::InspectorTarget::Value { owner, parameter }) => {
                        let root = this.root.clone();
                        this.session.update(cx, |session, cx| {
                            session.set_graph_value(root, owner, parameter, cx);
                        });
                    }
                    Some(super::graph_selection::InspectorTarget::Contents(id)) => {
                        if double && spectrum_data {
                            window.dispatch_action(
                                Box::new(super::menu::OpenInEditor { id: id.to_owned() }),
                                cx,
                            );
                        } else {
                            this.session.update(cx, |session, cx| {
                                if let Some(entry) = session.known_item(&id) {
                                    let project = session.project_value();
                                    session.select_contents(entry, project, cx);
                                }
                            });
                        }
                    }
                    None => {}
                }
            }))
            // Node interactions belong to the node. In particular, prevent
            // the graph's background pan gesture from starting before a
            // header or slot drag has crossed its drag threshold.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // A right-press opens the card's own menu below, whose handler
            // stops it there,
            // so the background's create menu never sees it.
            .can_drop(move |value, _, _| {
                value.downcast_ref::<SlotDrag>().is_some_and(|drag| {
                    graph_layout::can_connect(
                        &drop_graph,
                        &drop_node,
                        &drag.accepts,
                        drag.spectrum_data,
                        &drag.target,
                    )
                })
            })
            .drag_over::<SlotDrag>(|style, _, _, _| style.border_color(rgb(0x6fcf8a)))
            .on_drop(cx.listener(move |this, drag: &SlotDrag, _, cx| {
                this.connect(drag, &connect_node, cx)
            }))
            .context_menu({
                let session = self.session.clone();
                let id = menu_id.clone();
                let editor = editor.clone();
                move |mut menu, _, _| {
                    if !disconnects.is_empty() {
                        for action in disconnects.iter() {
                            let editor = editor.clone();
                            let edit = action.edit.clone();
                            let displaced = action.displaced.clone();
                            menu = menu.item(PopupMenuItem::new(action.label.clone()).on_click(
                                move |_, _, cx| {
                                    let _ = editor.update(cx, |this, cx| {
                                        if let Some(extra) = node_editor_create::displaced_resource(
                                            Some(&displaced),
                                            None,
                                        ) {
                                            this.remember_extra(&extra);
                                        }
                                        this.apply_edits(vec![edit.clone()], cx);
                                    });
                                },
                            ));
                        }
                        menu = menu.separator();
                    }
                    let open_id = id.clone();
                    menu = menu.item(PopupMenuItem::new("Open in Editor").on_click(
                        move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(super::menu::OpenInEditor {
                                    id: open_id.clone(),
                                }),
                                cx,
                            );
                        },
                    ));
                    if let Some(entry) = delete_entry.clone() {
                        let session = session.clone();
                        menu = menu
                            .separator()
                            .item(
                                PopupMenuItem::new("Delete…").on_click(move |_, window, cx| {
                                    let session_for_confirm = session.clone();
                                    super::project_actions::confirm_item_delete(
                                        session.clone(),
                                        entry.clone(),
                                        window,
                                        cx,
                                        move |edit, _, cx| {
                                            session_for_confirm.update(cx, |session, cx| {
                                                let project = session.project_value();
                                                if project.is_object()
                                                    && !session.document().is_read_only()
                                                {
                                                    session.send(
                                                        Command::Apply {
                                                            project,
                                                            edits: vec![edit.clone()],
                                                            follow: crate::backend::Follow::Nothing,
                                                            form: None,
                                                        },
                                                        cx,
                                                    );
                                                }
                                            });
                                        },
                                    );
                                }),
                            );
                    }
                    menu
                }
            })
            .into_any_element()
    }

    /// One SpectrumData node's single small colour swatch — the
    /// wire's chromaticity of its curve (the curve's magnitude-free colour,
    /// magnitude-free, the same box the Contents thumbnail and the
    /// Spectrum Data editor show), asked through the session's
    /// per-revision cache (`project.spectrum_color` behind the backend,
    /// off the UI thread), muted until the answer lands and on a refused
    /// read (a deleted resource answers an error, never a crash).
    fn spectrum_swatch(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let answer = self.session.update(cx, |session, cx| {
            session.spectrum_swatch(id, "chromaticity", cx)
        });
        swatch_element(answer, (HEADER - 24.) * self.zoom)
    }
}

/// The role a value node's swatch asks: a metaless name is what the
/// value node carries, so the metal rows' shapes pick — eta/k show their
/// chromaticity, a reflectance its albedo, everything else the schema's
/// own read.
fn value_node_role(parameter: &str) -> Option<String> {
    match parameter {
        "eta" | "k" | "conductor.eta" | "conductor.k" => Some("chromaticity".into()),
        "reflectance" => Some("albedo".into()),
        _ => None,
    }
}

/// One node swatch: the resolved colour filled in, or the shared hollow
/// look while the answer is on its way or was refused:
/// the filled muted dark read as "0"; hollow reads as nothing). The
/// The Inspector's spectrum summary shows the same swatch — one
/// painter, not a copy.
pub(super) fn swatch_element(answer: Option<Value>, edge: f32) -> AnyElement {
    let colour = answer
        .as_ref()
        .filter(|answer| answer.get("error").is_none())
        .map(|answer| {
            let channel = |index: usize| {
                answer["rgb"]
                    .as_array()
                    .and_then(|values| values.get(index))
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or_default()
            };
            Rgba {
                r: channel(0).clamp(0., 1.) as f32,
                g: channel(1).clamp(0., 1.) as f32,
                b: channel(2).clamp(0., 1.) as f32,
                a: 1.,
            }
        });
    let mut swatch = div().flex_shrink_0().rounded_sm().size(px(edge));
    swatch = if let Some(colour) = colour {
        swatch.bg(colour)
    } else {
        // Clipped to the box's rounding so the hollow outline keeps the
        // fill's silhouette.
        swatch
            .overflow_hidden()
            .child(super::components::unset_swatch())
    };
    swatch.into_any_element()
}

/// The background create menu —
/// `New Material ▸ <implementations>` and
/// `New Texture ▸ Float | Spectrum ▸ <implementations>` — the same
/// implementation lists as Contents ▸ New (`node_editor_create::menu_rows`
/// reads the same backend schema), whatever the editor's root is.
/// Every item asks the editor that opened the menu, which places the
/// node at the click that opened it.
fn create_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
    rows: &CreateMenu,
    editor: &WeakEntity<NodeEditor>,
) -> PopupMenu {
    let (material_editor, material) = (editor.clone(), rows.material.clone());
    let menu = menu.submenu("New Material", window, cx, move |menu, _, _| {
        create_rows(menu, &material_editor, "material", None, &material)
    });
    let (texture_editor, float, spectrum, spectrum_editor) = (
        editor.clone(),
        rows.texture_float.clone(),
        rows.texture_spectrum.clone(),
        editor.clone(),
    );
    menu.submenu("New Texture", window, cx, move |menu, window, cx| {
        let (float_editor, float) = (texture_editor.clone(), float.clone());
        let menu = menu.submenu("Float", window, cx, move |menu, _, _| {
            create_rows(menu, &float_editor, "texture", Some("float"), &float)
        });
        let (spectrum_editor, spectrum) = (spectrum_editor.clone(), spectrum.clone());
        menu.submenu("Spectrum", window, cx, move |menu, _, _| {
            create_rows(
                menu,
                &spectrum_editor,
                "texture",
                Some("spectrum"),
                &spectrum,
            )
        })
    })
}

/// One implementation list's menu rows: each item creates through this
/// editor (`create_from_menu`), whose apply edits the same
/// `create_resource` edit Contents ▸ New applies.
fn create_rows(
    menu: PopupMenu,
    editor: &WeakEntity<NodeEditor>,
    kind: &'static str,
    value_type: Option<&'static str>,
    implementations: &[String],
) -> PopupMenu {
    let mut menu = menu;
    for implementation in implementations {
        let (editor, implementation) = (editor.clone(), implementation.clone());
        menu = menu.item(
            PopupMenuItem::new(implementation.clone()).on_click(move |_, _, cx| {
                let _ = editor.update(cx, |editor, cx| {
                    editor.create_from_menu(kind, &implementation, value_type, cx)
                });
            }),
        );
    }
    menu
}

/// A menu item's edits, applied through the session: menu clicks run
/// outside the editor entity's listeners, so the apply goes the way
/// every Inspector menu sends — one transaction, one Undo step.
fn menu_apply(session: &Entity<Session>, edits: &[Value], cx: &mut App) {
    session.update(cx, |session, cx| {
        if edits.is_empty() {
            return;
        }
        let project = session.project_value();
        if project.is_object() && !session.document().is_read_only() {
            session.send(
                Command::Apply {
                    project,
                    edits: edits.to_vec(),
                    follow: crate::backend::Follow::Nothing,
                    form: None,
                },
                cx,
            );
        }
    });
}

impl NodeEditor {
    /// The value node's card — the slot's value, not a resource
    /// the approximate colour its value resolves to and the
    /// kind label the core summary spells ("RGB", "Blackbody 6500 K",
    /// "Named: stdillum-D65"), the parameter in the subtitle, no curve
    /// sample and no input rows. The menu holds the replacement
    /// actions: create a spectrum texture, use a SpectrumData, or
    /// disconnect the declared value back to r4's default — the last
    /// only when the table's rows say the operand has a default to
    /// return to — each in one undoable apply.
    fn value_node_box(
        &self,
        node: &Value,
        layout: &graph_layout::NodeBox,
        card: NodeCard,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let zoom = self.zoom;
        let selected = card.highlighted.as_deref() == Some(layout.id.as_str());
        let id = layout.id.clone();
        let clicked = node.clone();
        let swatch = self.value_swatch(node, cx);
        let owner = node["owner"].as_str().unwrap_or_default().to_owned();
        let parameter = node["parameter"].as_str().unwrap_or_default().to_owned();
        let summary = node["summary"].as_str().unwrap_or_default().to_owned();
        // The menu's own inputs: the slot's declaration, this
        // revision's items and the implementations a new texture of
        // the slot's lineage may take.
        // A Viewer shows the card without the menu — every action
        // would be refused anyway, and the placement button beside the
        // slots carries the same rule.
        let actions = (!card.read_only)
            .then(|| {
                let declared =
                    super::graph_selection::slot_declaration(&card.graph, &owner, &parameter);
                let (items, implementations) = {
                    let session = self.session.read(cx);
                    let implementations = super::resource_menu::implementations(
                        &session.resource_schema["texture_spectrum"],
                    );
                    (session.current_items(), implementations)
                };
                let target = json!({"kind":"resource","id":owner});
                declared.as_ref().map(|(name, value_type, required)| {
                    parameter_rows::value_node_menu(
                        &target,
                        (name.as_str(), value_type.as_str()),
                        *required,
                        &items,
                        &implementations,
                    )
                })
            })
            .flatten();
        let session = self.session.clone();
        div()
            .id(SharedString::from(format!("graph-node-{id}")))
            .absolute()
            .left(px(layout.x * zoom + self.pan.0))
            .top(px(layout.y * zoom + self.pan.1))
            .w(px(NODE_WIDTH * zoom))
            .h(px(layout.height * zoom))
            .text_size(px(12. * zoom))
            .bg(rgb(0x2a2d34))
            .border_1()
            .border_color(if selected {
                rgb(0x7fb2f0)
            } else {
                rgb(0x41444d)
            })
            .rounded_md()
            .overflow_hidden()
            .cursor_pointer()
            .child(
                div()
                    .id(SharedString::from(format!("graph-node-header-{id}")))
                    .h(px(HEADER * zoom))
                    .px(px(6. * zoom))
                    .flex()
                    .items_center()
                    .gap(px(4. * zoom))
                    .bg(rgb(0x33363e))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.drag_origin = Some(event.position);
                        }),
                    )
                    .child(swatch)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::BOLD)
                                    .child(summary),
                            )
                            .child(div().truncate().text_color(rgb(0x9098a6)).child(parameter)),
                    )
                    .when_some(actions, |this, actions| {
                        this.child(
                            Button::new(SharedString::from(format!("graph-value-menu-{id}")))
                                .icon(assets::IconName::EllipsisVertical)
                                .xsmall()
                                .accessibility_label("Value actions")
                                .dropdown_menu(move |mut menu, _, _| {
                                    for (label, edits) in actions.change_texture.clone() {
                                        let (session, edits) = (session.clone(), edits.clone());
                                        menu = menu.item(PopupMenuItem::new(label).on_click(
                                            move |_, _, cx| menu_apply(&session, &edits, cx),
                                        ));
                                    }
                                    if !actions.spectra.is_empty() {
                                        menu = menu.separator();
                                    }
                                    for (label, edits) in actions.spectra.clone() {
                                        let (session, edits) = (session.clone(), edits.clone());
                                        menu = menu.item(PopupMenuItem::new(label).on_click(
                                            move |_, _, cx| menu_apply(&session, &edits, cx),
                                        ));
                                    }
                                    if let Some(edits) = actions.disconnect.clone() {
                                        let (session, edits) = (session.clone(), edits);
                                        menu = menu.separator().item(
                                            PopupMenuItem::new("Disconnect").on_click(
                                                move |_, _, cx| menu_apply(&session, &edits, cx),
                                            ),
                                        );
                                    }
                                    menu
                                }),
                        )
                    })
                    .cursor_grab()
                    .on_drag(
                        NodeDrag {
                            id: layout.id.clone(),
                            x: layout.x,
                            y: layout.y,
                        },
                        move |_, _, _, cx| cx.new(|_| SlotLabel("Move node".to_owned())),
                    ),
            )
            // Selecting a value node drives the Inspector like any
            // other node.
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select(id.clone(), cx);
                window.dispatch_action(Box::new(super::menu::ShowInspector), cx);
                if let Some(super::graph_selection::InspectorTarget::Value { owner, parameter }) =
                    super::graph_selection::click_target(&clicked)
                {
                    let root = this.root.clone();
                    this.session.update(cx, |session, cx| {
                        session.set_graph_value(root, owner, parameter, cx);
                    });
                }
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // A right-press belongs to the card as well —
            // the create menu is the background's own.
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .into_any_element()
    }

    /// The value node's approximate colour — the wire's
    /// `project.spectrum_color` for its owner's parameter, asked
    /// through the session's per-revision cache; muted while the
    /// answer is on its way or was refused, like the SpectrumData
    /// node's swatch.: the role follows the slot — an eta/k value
    /// shows its chromaticity, a reflectance its albedo; anything else
    /// leaves the choice to the core's schema read.
    fn value_swatch(&self, node: &Value, cx: &mut Context<Self>) -> AnyElement {
        let target = json!({"kind":"resource","id":node["owner"]});
        let parameter = node["parameter"].as_str().unwrap_or_default().to_owned();
        let role = value_node_role(&parameter);
        let answer = self.session.update(cx, |session, cx| {
            session.parameter_swatch(&target, &parameter, role.as_deref(), cx)
        });
        swatch_element(answer, (HEADER - 24.) * self.zoom)
    }
}

impl Render for NodeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        // An older revision's graph can stay on screen until the new one
        // arrives: the values it shows edit through the Inspector.
        let root_graph = session
            .resource_graphs
            .get(&self.root)
            .map(|(_, graph)| graph.clone())
            .unwrap_or(Value::Null);
        // The graphs of the ids created here and still
        // unconnected merge in — the one graph the cards,
        // edges, previews, selection and the drop check all see, laid
        // out and drawn.
        let extras = self
            .extra
            .iter()
            .filter_map(|id| {
                session
                    .resource_graphs
                    .get(id)
                    .map(|(_, graph)| graph.clone())
            })
            .collect::<Vec<_>>();
        let graph = node_editor_create::merged_graph(&root_graph, &extras);
        let read_only = session.document().is_read_only();
        // The create menu offers the same implementation lists
        // Contents ▸ New builds from (the backend's static summary,
        // session state); a read-only document offers no menu at all.
        let create_menu_rows = if read_only {
            CreateMenu::default()
        } else {
            node_editor_create::menu_rows(&session.resource_schema)
        };
        // A drag released outside the editor never reached its handlers.
        if !cx.has_active_drag() {
            self.drag_line = None;
            self.drag_origin = None;
        }
        let mut layout = graph_layout::layout(&graph);
        if graph["nodes"].is_array() {
            // Only explicitly dragged positions override automatic placement.
            // Ids not in this graph are kept: a first, partial graph must not
            // drop positions read from the project, and the layout skips them.
            layout = graph_layout::with_owner_positions(layout, &graph, &self.positions);
        }
        // After a structural change, the first graph to
        // arrive included — fit the graph when a drawn card would fall
        // outside the view; an unmeasured viewport fits and
        // the owed fit retries next render. Same-set edits — a value
        // change — hold the view, so a manual pan stays put until the
        // set itself changes.
        let view = self.viewport.get().size;
        let view = (f32::from(view.width), f32::from(view.height));
        let drawn: HashSet<String> = layout.nodes.iter().map(|node| node.id.clone()).collect();
        let reframe = should_reframe(
            self.drawn.as_ref(),
            &drawn,
            &fit_flags(&layout.nodes, self.zoom, self.pan, view),
            self.frame_pending,
        );
        self.drawn = Some(drawn);
        if reframe {
            let was_pending = self.frame_pending;
            let framed = self.frame_all(&layout);
            self.frame_pending = !framed;
            // The fitted view draws now; a fit that found no viewport
            // schedules its single retry next render, not a loop.
            if framed || !was_pending {
                cx.notify();
            }
        }
        // The highlight follows the Inspector into this graph: the
        // value node its colour page names, or the Contents item it
        // shows — a selection anywhere else leaves none.
        let (value_target, inspector) = {
            let session = self.session.read(cx);
            (session.graph_value(), session.project_inspector.clone())
        };
        let highlighted =
            super::graph_selection::highlighted(&graph, &self.root, value_target, &inspector);
        // One shared handle: a card reads the graph's slot rows without
        // copying the whole graph per card per frame.
        let graph_handle = Rc::new(graph.clone());
        let nodes = graph["nodes"].as_array().cloned().unwrap_or_default();
        let boxes = layout
            .nodes
            .iter()
            .zip(&nodes)
            .map(|(layout, node)| {
                let kind = match node["kind"].as_str() {
                    Some("material") => Some(PreviewKind::Material),
                    Some("texture") => Some(PreviewKind::Texture),
                    _ => None,
                };
                // Only cards on screen ask: a graph has up to 256 nodes,
                // and offscreen previews would only fill the preview cache.
                let preview = kind
                    .filter(|_| on_screen(layout, self.zoom, self.pan, view))
                    .and_then(|kind| {
                        self.session
                            .update(cx, |session, cx| session.preview(&layout.id, kind, cx))
                    });
                self.node_box(
                    node,
                    layout,
                    &graph,
                    NodeCard {
                        graph: graph_handle.clone(),
                        read_only,
                        preview,
                        highlighted: highlighted.clone(),
                    },
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let (zoom, pan) = (self.zoom, self.pan);
        let edges = layout.edges.clone();
        let drag_line = self.drag_line;
        let viewport = self.viewport.clone();
        let edge_canvas = canvas(
            move |bounds, _, _| viewport.set(bounds),
            move |bounds, _, window, _| {
                let origin = bounds.origin;
                let at = |(x, y): (f32, f32)| {
                    point(
                        origin.x + px(x * zoom + pan.0),
                        origin.y + px(y * zoom + pan.1),
                    )
                };
                for edge in &edges {
                    let (from, to) = (at(edge.from), at(edge.to));
                    let bend = px((edge.to.0 - edge.from.0).abs().max(40.) * zoom / 2.);
                    let mut path = PathBuilder::stroke(px(1.5));
                    path.move_to(from);
                    path.cubic_bezier_to(
                        to,
                        point(from.x + bend, from.y),
                        point(to.x - bend, to.y),
                    );
                    if let Ok(path) = path.build() {
                        window.paint_path(path, rgb(0x7fb2f0));
                    }
                }
                // The slot being dragged, to the pointer.
                if let Some(((x1, y1), (x2, y2))) = drag_line {
                    let mut path = PathBuilder::stroke(px(1.5));
                    path.move_to(point(origin.x + px(x1), origin.y + px(y1)));
                    path.line_to(point(origin.x + px(x2), origin.y + px(y2)));
                    if let Ok(path) = path.build() {
                        window.paint_path(path, rgb(0x6fcf8a));
                    }
                }
            },
        )
        .absolute()
        .size_full();
        let frame_layout = layout.clone();
        let message = graph["error"].as_str().map(str::to_owned);
        let background = div()
            .id("node-editor-graph")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(edge_canvas)
            .children(boxes)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, _| {
                    this.last_mouse = Some(event.position);
                    this.drag_origin = Some(event.position);
                }),
            )
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<NodeDrag>, _, cx| {
                let drag = event.drag(cx);
                // Where this move measures from: the press point the
                // header recorded — which a render between the press
                // and the drag's first move may have cleared (a drag
                // activates only once the pointer crosses GPUI's
                // threshold; render drops the record while no drag
                // is active), in which case the move seeds it with
                // itself and the node travels on from there. The
                // seed persists for the drag's later moves, and it
                // never outlives the drag: render clears it once no
                // drag is active again.
                let origin = drag_move_origin(this.drag_origin, event.event.position);
                this.drag_origin = Some(origin);
                let moved = dragged_node_position(drag, origin, event.event.position, this.zoom);
                this.positions.insert(drag.id.clone(), moved);
                let root = this.root.clone();
                let id = drag.id.clone();
                this.session.update(cx, |session, _| {
                    session.remember_node_position(&root, &id, moved);
                });
                cx.notify();
            }))
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<SlotDrag>, _, cx| {
                let from = event.drag(cx).from;
                let origin = this.viewport.get().origin;
                this.drag_line = Some((
                    (
                        from.0 * this.zoom + this.pan.0,
                        from.1 * this.zoom + this.pan.1,
                    ),
                    (
                        f32::from(event.event.position.x - origin.x),
                        f32::from(event.event.position.y - origin.y),
                    ),
                ));
                cx.notify();
            }))
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<OutputDrag>, _, cx| {
                    let from = event.drag(cx).from;
                    let origin = this.viewport.get().origin;
                    this.drag_line = Some((
                        (
                            from.0 * this.zoom + this.pan.0,
                            from.1 * this.zoom + this.pan.1,
                        ),
                        (
                            f32::from(event.event.position.x - origin.x),
                            f32::from(event.event.position.y - origin.y),
                        ),
                    ));
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.drag_origin = None;
                    if this.drag_line.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.drag_origin = None;
                    if this.drag_line.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                // A slot drag moves the line, not the view.
                if cx.has_active_drag() {
                    this.last_mouse = None;
                    return;
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    this.last_mouse = None;
                    return;
                }
                if let Some(last) = this.last_mouse.replace(event.position) {
                    this.pan.0 += f32::from(event.position.x - last.x);
                    this.pan.1 += f32::from(event.position.y - last.y);
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
                let zoom = (this.zoom * (1. + delta / 400.)).clamp(0.25, 2.);
                // Keep the point under the cursor where it is.
                let origin = this.viewport.get().origin;
                let cursor = (
                    f32::from(event.position.x - origin.x),
                    f32::from(event.position.y - origin.y),
                );
                let scale = zoom / this.zoom;
                this.pan = (
                    cursor.0 - (cursor.0 - this.pan.0) * scale,
                    cursor.1 - (cursor.1 - this.pan.1) * scale,
                );
                this.zoom = zoom;
                cx.notify();
            }));
        // The create menu is the background's own — never
        // in a read-only document, and a card's right-press never reaches
        // it. The press remembers its point in
        // graph units before the menu opens, so the created node lands
        // under it.
        let background = if read_only {
            background.into_any_element()
        } else {
            background
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, _, _| {
                        let origin = this.viewport.get().origin;
                        this.pending_create = Some(node_editor_create::graph_point(
                            event.position,
                            origin,
                            this.pan,
                            this.zoom,
                        ));
                    }),
                )
                .context_menu({
                    let editor = cx.entity().downgrade();
                    let rows = create_menu_rows;
                    move |menu, window, cx| create_menu(menu, window, cx, &rows, &editor)
                })
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x17191e))
            .text_color(rgb(0xd4d7dd))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .bg(rgb(0x25272d))
                    .child(
                        Button::new("node-editor-frame-all")
                            .label("Frame All")
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.frame_all(&frame_layout);
                                cx.notify()
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x9098a6))
                            .child(if read_only {
                                "Read-only viewer · values and connections are disabled · header drag to move · background drag to pan · wheel to zoom"
                            } else {
                                "Upstream only · header drag to move · background drag to pan · wheel to zoom"
                            }),
                    )
                    .when(graph["truncated"] == true, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xd0a86a))
                                .child("Large graph: some upstream nodes are not shown"),
                        )
                    })
                    .when_some(message, |this, message| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xd99a74))
                                .child(message),
                        )
                    }),
            )
            .child(background)
    }
}

/// Whether a node card overlaps the viewport (`view` = its width, height),
/// in the same pan/zoom placement `node_box` draws. Before the first paint
/// the viewport is unknown (zero), so every card counts as on screen.
fn on_screen(layout: &graph_layout::NodeBox, zoom: f32, pan: (f32, f32), view: (f32, f32)) -> bool {
    if view.0 <= 0. || view.1 <= 0. {
        return true;
    }
    let (left, top) = (layout.x * zoom + pan.0, layout.y * zoom + pan.1);
    left < view.0
        && left + NODE_WIDTH * zoom > 0.
        && top < view.1
        && top + layout.height * zoom > 0.
}

/// Whether this render should fit the graph into the viewport before
/// drawing it (S4 item 8): when the node set drawn last changed — the
/// first graph to arrive counts — and a card would fall outside the
/// view, or when an earlier fit (`frame_pending`) still waits for the
/// viewport's size. Value-only edits keep the same set, so they never
/// move the view, a manual pan included; the pan stays put until the
/// node set itself changes.
fn should_reframe(
    previous: Option<&HashSet<String>>,
    current: &HashSet<String>,
    on_screen: &[bool],
    frame_pending: bool,
) -> bool {
    frame_pending || (previous != Some(current) && on_screen.iter().any(|visible| !visible))
}

/// The on-screen answers the fit decision uses: a viewport that has
/// never been measured vouches for no card — before the canvas is
/// first painted every card counts as off screen, so the fit runs,
/// finds nothing to fit into, and retries once the size exists.
fn fit_flags(
    nodes: &[graph_layout::NodeBox],
    zoom: f32,
    pan: (f32, f32),
    view: (f32, f32),
) -> Vec<bool> {
    if view.0 <= 0. || view.1 <= 0. {
        return vec![false; nodes.len()];
    }
    nodes
        .iter()
        .map(|node| on_screen(node, zoom, pan, view))
        .collect()
}

/// The origin a node drag's moves measure their travel from. The
/// header's mouse-down records the press point, but GPUI activates a
/// drag only after the pointer crosses its start threshold, and every
/// render in that window — the press's own refresh among them — clears
/// the record (render drops it while no drag is active). A drag that
/// arrives with no record seeds it with the first move's own position,
/// so the node still moves: it holds its place and follows the pointer
/// from there.
fn drag_move_origin(recorded: Option<Point<Pixels>>, seed: Point<Pixels>) -> Point<Pixels> {
    recorded.unwrap_or(seed)
}

/// Where a dragged node lands: the position the drag carried plus the
/// pointer's travel from the drag's origin, back in graph units.
fn dragged_node_position(
    drag: &NodeDrag,
    origin: Point<Pixels>,
    pointer: Point<Pixels>,
    zoom: f32,
) -> (f32, f32) {
    (
        drag.x + f32::from(pointer.x - origin.x) / zoom,
        drag.y + f32::from(pointer.y - origin.y) / zoom,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        NODE_WIDTH, NodeDrag, OutputDrag, SlotDrag, connection_edits, drag_move_origin,
        dragged_node_position, fit_flags, graph_layout, on_screen, output_can_connect,
        should_reframe,
    };
    use gpui_kit::{point, px};
    use serde_json::{Value, json};
    use std::collections::HashSet;

    fn card(x: f32, y: f32) -> graph_layout::NodeBox {
        graph_layout::NodeBox {
            id: "n".into(),
            x,
            y,
            height: 100.,
            inputs: Vec::new(),
        }
    }

    fn ids(names: &[&str]) -> HashSet<String> {
        names.iter().copied().map(str::to_owned).collect()
    }

    #[test]
    fn output_and_input_drags_share_acceptance_rules() {
        let graph = json!({"edges":[]});
        let output = OutputDrag {
            id: "tint".into(),
            kind: "texture".into(),
            texture_value_type: "spectrum".into(),
            from: (0., 0.),
        };
        let source = output.node();
        assert_eq!(
            output_can_connect(&graph, &output, "spectrum", false, "wall"),
            graph_layout::can_connect(&graph, &source, "spectrum", false, "wall")
        );
        assert!(output_can_connect(
            &graph, &output, "spectrum", false, "wall"
        ));
        assert!(!output_can_connect(&graph, &output, "float", false, "wall"));
        assert!(!output_can_connect(
            &graph, &output, "spectrum", false, "tint"
        ));
        let material = OutputDrag {
            id: "coat".into(),
            kind: "material".into(),
            texture_value_type: String::new(),
            from: (0., 0.),
        };
        assert!(output_can_connect(
            &graph, &material, "material", false, "mix"
        ));
        assert!(!output_can_connect(
            &graph, &material, "material", false, "coat"
        ));
    }

    #[test]
    fn output_row_drop_builds_the_same_edit_as_input_node_drop() {
        let drag = SlotDrag {
            target: "mix".into(),
            parameter: "materials".into(),
            index: Some(1),
            accepts: "material".into(),
            materials: vec!["a".into(), "b".into()],
            previous: Some("b".into()),
            spectrum_data: false,
            required: true,
            declared: None,
            from: (0., 0.),
        };
        let output = OutputDrag {
            id: "coat".into(),
            kind: "material".into(),
            texture_value_type: String::new(),
            from: (0., 0.),
        };
        let forward_source = output.node();
        let reverse_source: Value = json!({"id":"coat","kind":"material"});
        assert_eq!(
            connection_edits(&drag, &forward_source),
            connection_edits(&drag, &reverse_source)
        );
        assert_eq!(
            connection_edits(&drag, &forward_source)[0]["value"]["values"]["value"]["ids"],
            json!(["a", "coat"])
        );
    }

    #[test]
    fn only_cards_overlapping_the_viewport_are_on_screen() {
        let view = (400., 300.);
        assert!(on_screen(&card(10., 10.), 1., (0., 0.), view));
        // Partly visible on each edge still counts.
        assert!(on_screen(&card(-NODE_WIDTH + 1., 10.), 1., (0., 0.), view));
        assert!(on_screen(&card(10., 299.), 1., (0., 0.), view));
        // Fully outside on each side does not.
        assert!(!on_screen(&card(-NODE_WIDTH - 1., 10.), 1., (0., 0.), view));
        assert!(!on_screen(&card(401., 10.), 1., (0., 0.), view));
        assert!(!on_screen(&card(10., -101.), 1., (0., 0.), view));
        assert!(!on_screen(&card(10., 301.), 1., (0., 0.), view));
        // Pan and zoom place the card the way node_box draws it.
        assert!(!on_screen(&card(500., 10.), 1., (0., 0.), view));
        assert!(on_screen(&card(500., 10.), 1., (-300., 0.), view));
        assert!(on_screen(&card(500., 10.), 0.5, (0., 0.), view));
        // An unknown viewport asks for everything once.
        assert!(on_screen(&card(5000., 5000.), 1., (0., 0.), (0., 0.)));
    }

    #[test]
    fn structural_changes_reframe_only_when_a_card_falls_off_screen() {
        let one = ids(&["a"]);
        let two = ids(&["a", "b"]);
        // The first graph to arrive counts as a change; so does any
        // node added or removed.
        assert!(!should_reframe(None, &one, &[true], false));
        assert!(should_reframe(None, &one, &[false], false));
        assert!(!should_reframe(Some(&one), &two, &[true, true], false));
        assert!(should_reframe(Some(&two), &one, &[true, false], false));
    }

    #[test]
    fn same_set_edits_keep_the_users_view() {
        let one = ids(&["a"]);
        // Value-only edits keep the node set, and never move the view —
        // even with every card off screen, which a manual pan is.
        assert!(!should_reframe(Some(&one), &one, &[false], false));
    }

    #[test]
    fn an_owed_fit_retries_regardless_of_the_set() {
        // A fit that could not run retries
        // on the next render, whatever the set comparison says.
        let one = ids(&["a"]);
        assert!(should_reframe(Some(&one), &one, &[true], true));
    }

    #[test]
    fn an_unmeasured_viewport_vouches_for_no_card() {
        // Before the first paint the view is unknown — the fit flags
        // read "off screen" so the fit runs and, failing, retries; a
        // measured view answers per card.
        assert_eq!(
            fit_flags(&[card(10., 10.)], 1., (0., 0.), (0., 0.)),
            vec![false]
        );
        assert_eq!(
            fit_flags(
                &[card(10., 10.), card(5000., 10.)],
                1.,
                (0., 0.),
                (400., 300.)
            ),
            vec![true, false]
        );
        assert_eq!(fit_flags(&[], 1., (0., 0.), (0., 0.)), Vec::<bool>::new());
    }

    #[test]
    fn a_node_still_moves_when_a_rerender_between_press_and_move_cleared_the_origin() {
        let drag = NodeDrag {
            id: "conductor".into(),
            x: 10.,
            y: 20.,
        };
        // GPUI activates a drag only once the pointer has crossed its
        // start threshold, and the renders in that window clear the
        // recorded press point (render drops it while no drag is
        // active). The first move seeds the missing origin with itself,
        // so the node holds its place right there...
        let seeded = drag_move_origin(None, point(px(106.), px(54.)));
        assert_eq!(seeded, point(px(106.), px(54.)));
        assert_eq!(
            dragged_node_position(&drag, seeded, point(px(106.), px(54.)), 1.),
            (10., 20.)
        );
        // ...and travels once the pointer moves on, in graph units.
        assert_eq!(
            dragged_node_position(&drag, seeded, point(px(150.), px(80.)), 2.),
            (32., 33.)
        );
        // A press point that survived to the drag still anchors it.
        let pressed = drag_move_origin(Some(point(px(100.), px(50.))), point(px(106.), px(54.)));
        assert_eq!(pressed, point(px(100.), px(50.)));
        assert_eq!(
            dragged_node_position(&drag, pressed, point(px(150.), px(80.)), 1.),
            (60., 50.)
        );
    }
}
