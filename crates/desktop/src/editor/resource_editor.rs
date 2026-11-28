//! A dedicated resource editor tab. The Image Viewer
//! decodes the asset file off the GPUI thread and shows it read-only; the
//! Material / Texture Editor hosts the node editor.
use super::editor_panels::{self, EditorKind};
use super::image_display;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{Sizable, spinner::Spinner};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use std::sync::Arc;

fn target_response_is_current(
    requested_generation: u64,
    current_generation: u64,
    requested_id: Option<&str>,
    current_id: Option<&str>,
) -> bool {
    requested_generation == current_generation && requested_id == current_id
}

pub struct ResourceEditor {
    kind: EditorKind,
    session: Entity<super::Session>,
    switch_generation: u64,
    /// The Contents entry this tab edits; its `id` names the tab.
    pub entry: Value,
    title: String,
    target_loading: bool,
    focus: FocusHandle,
    image: Option<(Arc<RenderImage>, [u32; 2])>,
    /// Set when the asset needed the shared tone map (HDR): the caption
    /// says the display is tone mapped, not the stored data.
    tone_mapped: bool,
    message: String,
    graph: Option<Entity<super::node_editor::NodeEditor>>,
    mesh: Option<Entity<super::mesh_viewer::MeshViewer>>,
    array: Option<Entity<super::array_editor::ArrayEditor>>,
    spectrum: Option<Entity<super::spectrum_editor::SpectrumEditor>>,
}

impl ResourceEditor {
    pub fn new(
        kind: EditorKind,
        entry: Value,
        path: Option<String>,
        session: Entity<super::Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = editor_panels::title(kind, &entry);
        let has_target = entry["id"].as_str().is_some();
        let graph = (kind.is_graph() && has_target).then(|| {
            let root = entry["id"].as_str().unwrap_or_default().to_owned();
            cx.new(|cx| super::node_editor::NodeEditor::new(session.clone(), root, cx))
        });
        let array = (kind == EditorKind::ArrayEditor && has_target).then(|| {
            let session = session.clone();
            let entry = entry.clone();
            cx.new(|cx| super::array_editor::ArrayEditor::new(session, &entry, window, cx))
        });
        // Spectrum Data resources have their own editor panel.
        let spectrum = (kind == EditorKind::SpectrumEditor && has_target).then(|| {
            let session = session.clone();
            let entry = entry.clone();
            cx.new(|cx| super::spectrum_editor::SpectrumEditor::new(session, &entry, window, cx))
        });
        // The viewer parses and draws the mesh itself.
        let (mesh, path) = if kind == EditorKind::MeshViewer && has_target {
            let (entry, path) = (entry.clone(), path);
            (
                Some(cx.new(|cx| {
                    super::mesh_viewer::MeshViewer::new(session.clone(), entry, path, window, cx)
                })),
                None,
            )
        } else {
            (None, path)
        };
        let target_loading = kind == EditorKind::ImageViewer && path.is_some();
        let initial_generation = 0;
        let initial_id = entry["id"].as_str().map(str::to_owned);
        if let Some(path) = path {
            let media_type = entry["media_type"].as_str().unwrap_or_default().to_owned();
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move { decode(&path, &media_type) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if !target_response_is_current(
                        initial_generation,
                        this.switch_generation,
                        initial_id.as_deref(),
                        this.item_id(),
                    ) {
                        return;
                    }
                    match decoded {
                        Ok((image, size, tone_mapped)) => {
                            this.image = Some((image, size));
                            this.tone_mapped = tone_mapped;
                        }
                        Err(error) => this.message = error,
                    }
                    this.target_loading = false;
                    cx.notify();
                });
            })
            .detach();
        }
        Self {
            kind,
            session,
            switch_generation: 0,
            entry,
            title,
            target_loading,
            focus: cx.focus_handle(),
            image: None,
            tone_mapped: false,
            message: if has_target {
                "Loading…"
            } else {
                "Select an item from Contents"
            }
            .into(),
            graph,
            mesh,
            array,
            spectrum,
        }
    }

    /// Change the target while retaining this panel's Dock identity/location.
    /// Kind is fixed for resource editors; ArrayEditors remain distinct forms.
    pub fn switch_target(
        &mut self,
        entry: Value,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        debug_assert_eq!(editor_panels::editor_for(&entry), Some(self.kind));
        self.switch_generation = self.switch_generation.wrapping_add(1);
        let generation = self.switch_generation;
        let item_id = entry["id"].as_str().unwrap_or_default().to_owned();
        self.entry = entry.clone();
        self.title = editor_panels::title(self.kind, &entry);
        self.drop_current_image(window);
        self.tone_mapped = false;
        self.message = "Loading…".into();
        self.target_loading = true;
        self.graph = self.kind.is_graph().then(|| {
            let root = item_id.clone();
            let session = self.session.clone();
            cx.new(|cx| super::node_editor::NodeEditor::new(session, root, cx))
        });
        self.array = (self.kind == EditorKind::ArrayEditor).then(|| {
            let session = self.session.clone();
            let editor_entry = entry.clone();
            cx.new(|cx| super::array_editor::ArrayEditor::new(session, &editor_entry, window, cx))
        });
        self.spectrum = (self.kind == EditorKind::SpectrumEditor).then(|| {
            let session = self.session.clone();
            let editor_entry = entry.clone();
            cx.new(|cx| {
                super::spectrum_editor::SpectrumEditor::new(session, &editor_entry, window, cx)
            })
        });
        self.mesh = if self.kind == EditorKind::MeshViewer {
            let entry = entry.clone();
            let path = path.clone();
            let session = self.session.clone();
            Some(cx.new(|cx| super::mesh_viewer::MeshViewer::new(session, entry, path, window, cx)))
        } else {
            None
        };
        if self.kind == EditorKind::MeshViewer {
            self.target_loading = false;
            cx.notify();
            return;
        }
        if self.kind == EditorKind::ArrayEditor {
            self.target_loading = false;
            cx.notify();
            return;
        }
        if self.kind == EditorKind::SpectrumEditor {
            self.target_loading = false;
            cx.notify();
            return;
        }
        let Some(path) = path else {
            self.message = "Select an item from Contents".into();
            self.target_loading = false;
            cx.notify();
            return;
        };
        let media_type = entry["media_type"].as_str().unwrap_or_default().to_owned();
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move { decode(&path, &media_type) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !target_response_is_current(
                    generation,
                    this.switch_generation,
                    Some(item_id.as_str()),
                    this.item_id(),
                ) {
                    return;
                }
                match decoded {
                    Ok((image, size, tone_mapped)) => {
                        this.image = Some((image, size));
                        this.tone_mapped = tone_mapped;
                    }
                    Err(error) => this.message = error,
                }
                this.target_loading = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn begin_switch(&mut self, entry: Value, window: &mut Window, cx: &mut Context<Self>) {
        debug_assert_eq!(editor_panels::editor_for(&entry), Some(self.kind));
        self.switch_generation = self.switch_generation.wrapping_add(1);
        self.entry = entry.clone();
        self.title = editor_panels::title(self.kind, &entry);
        self.drop_current_image(window);
        self.tone_mapped = false;
        self.message = "Loading…".into();
        self.target_loading = true;
        self.graph = None;
        self.mesh = None;
        self.array = None;
        self.spectrum = None;
        cx.notify();
    }

    pub fn show_open_error(
        &mut self,
        entry: Value,
        error: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_switch(entry, window, cx);
        self.message = error;
        self.target_loading = false;
        cx.notify();
    }

    pub fn kind(&self) -> EditorKind {
        self.kind
    }

    pub fn title_for_menu(&self) -> String {
        self.title.clone()
    }

    pub fn is_loading(&self) -> bool {
        self.target_loading
    }

    pub fn clear_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_generation = self.switch_generation.wrapping_add(1);
        self.entry = Value::Null;
        self.title = self.kind.name().into();
        self.drop_current_image(window);
        self.tone_mapped = false;
        self.message = "Select an item from Contents".into();
        self.target_loading = false;
        self.graph = None;
        self.mesh = None;
        self.array = None;
        self.spectrum = None;
        cx.notify();
    }

    fn drop_current_image(&mut self, window: &mut Window) {
        if let Some((image, _)) = self.image.take() {
            let _ = window.drop_image(image);
        }
    }

    pub fn item_id(&self) -> Option<&str> {
        self.entry["id"].as_str()
    }
}

/// Decode an image asset to a GPUI image (GPUI wants BGRA8): the shared
/// pipeline in `image_display` does the size bound and the tone map.
fn decode(path: &str, media_type: &str) -> Result<(Arc<RenderImage>, [u32; 2], bool), String> {
    let (mut image, tone_mapped) = image_display::decode_to_rgba(path, media_type)?;
    let size = [image.width(), image.height()];
    for pixel in image.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok((
        Arc::new(RenderImage::new(vec![image::Frame::new(image)])),
        size,
        tone_mapped,
    ))
}

impl BasePanel for ResourceEditor {
    fn panel_name(&self) -> &'static str {
        editor_panels::PANEL_NAME
    }
}

impl Panel for ResourceEditor {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }
}

impl EventEmitter<PanelEvent> for ResourceEditor {}

impl Focusable for ResourceEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ResourceEditor {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if let Some(graph) = &self.graph {
            return div()
                .track_focus(&self.focus)
                .size_full()
                .child(graph.clone())
                .into_any_element();
        }
        if let Some(mesh) = &self.mesh {
            return div()
                .track_focus(&self.focus)
                .size_full()
                .child(mesh.clone())
                .into_any_element();
        }
        if let Some(array) = &self.array {
            return div().size_full().child(array.clone()).into_any_element();
        }
        if let Some(spectrum) = &self.spectrum {
            return div().size_full().child(spectrum.clone()).into_any_element();
        }
        let caption = match &self.image {
            Some((_, [width, height])) => {
                let mut text = format!(
                    "{} · {width} × {height} · read-only",
                    self.entry["location"].as_str().unwrap_or_default()
                );
                if self.tone_mapped {
                    text.push_str(" · ");
                    text.push_str(image_display::TONE_MAPPED_LABEL);
                }
                text
            }
            None => self.message.clone(),
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x17191e))
            .text_color(rgb(0xd4d7dd))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(self.image.as_ref().map(|(image, _)| {
                        img(image.clone())
                            .size_full()
                            .object_fit(ObjectFit::Contain)
                    })),
            )
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .bg(rgb(0x25272d))
                    .text_color(rgb(0x9098a6))
                    .flex()
                    .items_center()
                    .gap_2()
                    // Non-blocking status: a small spinner beside the
                    // "Loading…" caption while the decode runs.
                    .when(self.target_loading, |row| {
                        row.child(Spinner::new().xsmall())
                    })
                    .child(caption),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::target_response_is_current;

    #[test]
    fn image_decode_from_previous_target_cannot_replace_current_target() {
        assert!(target_response_is_current(7, 7, Some("B"), Some("B")));
        assert!(!target_response_is_current(6, 7, Some("A"), Some("B")));
        assert!(!target_response_is_current(7, 7, Some("A"), Some("B")));
    }
}
