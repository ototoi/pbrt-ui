//! Apply display updates to the editor session on the GPUI thread.
use super::Session;
use super::contents_thumbnails::ThumbnailId;
use crate::backend::{self, Backend, Command, PreviewKind, Update};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::*;
use scene_view::PreviewMode;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) fn same_preview_owner(
    old_project: Option<&protocol::project::ProjectRef>,
    old_scene: Option<protocol::SceneId>,
    new_project: Option<&protocol::project::ProjectRef>,
    new_scene: Option<protocol::SceneId>,
) -> bool {
    old_project.zip(new_project).is_some_and(|(old, new)| {
        old.project_id == new.project_id
            && old.activation == new.activation
            && old_scene.is_some()
            && old_scene == new_scene
    })
}

pub(super) fn scene_geometry_interactive(updating: bool, stale_frame: bool) -> bool {
    !updating && !stale_frame
}

pub(super) fn record_status(status: &mut String, messages: &mut Vec<String>, next: String) {
    if *status != next {
        *status = next.clone();
        messages.push(next);
        if messages.len() > 200 {
            messages.remove(0);
        }
    }
}

fn clear_project_tree_on_activation_change(tree: &mut serde_json::Value, changed: bool) {
    if changed {
        *tree = serde_json::json!({"nodes": []});
    }
}

#[cfg(test)]
mod status_tests {
    use super::{clear_project_tree_on_activation_change, record_status};

    #[test]
    fn project_activation_discards_the_previous_hierarchy_tree() {
        let mut tree = serde_json::json!({"nodes":[{"id":"old-scene"}]});
        clear_project_tree_on_activation_change(&mut tree, true);
        assert_eq!(tree, serde_json::json!({"nodes":[]}));

        let current = serde_json::json!({"nodes":[{"id":"current-scene"}]});
        tree = current.clone();
        clear_project_tree_on_activation_change(&mut tree, false);
        assert_eq!(tree, current);
    }

    #[test]
    fn status_is_logged_once_and_history_is_capped() {
        let mut status = String::new();
        let mut messages = Vec::new();

        record_status(&mut status, &mut messages, "waiting".into());
        record_status(&mut status, &mut messages, "waiting".into());
        assert_eq!(messages, ["waiting"]);

        for index in 0..200 {
            record_status(&mut status, &mut messages, format!("status {index}"));
        }
        assert_eq!(messages.len(), 200);
        assert_eq!(messages.first().map(String::as_str), Some("status 0"));
        assert_eq!(status, "status 199");
    }
}

/// One texture's or material's preview: the revision it was last asked at
/// (Null until first asked, and again after a Busy backoff or an LRU
/// evict), a 250ms Busy backoff deadline, the decoded image kept until a
/// newer one lands (none while a failure is a no-show), when a panel
/// last drew it, and the content key of the held image (a reply with
/// the same key keeps it — the PNG is not read again).
#[derive(Default)]
pub(super) struct Preview {
    at: serde_json::Value,
    retry_at: Option<Instant>,
    image: Option<Arc<RenderImage>>,
    used: Option<Instant>,
    key: Option<String>,
}

/// One SpectrumData resource's backend answers: the whole sample
/// table and the swatch colour per role, each asked once per project.
#[derive(Default)]
pub(super) struct SpectrumAnswers {
    /// The project reference everything here was asked at.
    at: serde_json::Value,
    table: Asked<Result<serde_json::Value, String>>,
    swatches: std::collections::BTreeMap<String, Asked<Result<serde_json::Value, String>>>,
}

/// One answer's single-flight state.
#[derive(Default)]
enum Asked<T> {
    #[default]
    Unasked,
    Pending,
    Done(T),
}

impl<T> Asked<T> {
    fn asked(&self) -> bool {
        !matches!(self, Self::Unasked)
    }
    fn done(&self) -> Option<&T> {
        match self {
            Self::Done(value) => Some(value),
            _ => None,
        }
    }
}

impl SpectrumAnswers {
    fn current(&self, at: &serde_json::Value) -> bool {
        self.at == *at
    }
    fn reset(&mut self, at: serde_json::Value) {
        self.at = at;
        self.table = Asked::Unasked;
        self.swatches.clear();
    }

    /// Whether `slot`'s answer needs its one command queued for
    /// `project` now: a new project resets every slot (their landed
    /// answers dropped, not shown for the wrong revision), and a slot
    /// asked at this revision — pending or landed — never queues a
    /// second command. The caller marks the slot pending only once the
    /// queue accepted, so a dropped command asks again next render.
    pub(super) fn asking(&mut self, project: &serde_json::Value, slot: &str) -> bool {
        if !self.current(project) {
            self.reset(project.clone());
        }
        !self.swatches.get(slot).is_some_and(|asked| asked.asked())
    }

    /// The queue accepted the slot's command; the next renders wait
    /// for the answer instead of queueing again.
    pub(super) fn mark_pending(&mut self, slot: &str) {
        self.swatches.insert(slot.to_owned(), Asked::Pending);
    }

    /// The slot's landed answer, with a refused read surfacing as
    /// `{error}` like every other backend answer.
    pub(super) fn swatch_answer(&self, slot: &str) -> Option<serde_json::Value> {
        self.swatches
            .get(slot)?
            .done()
            .cloned()
            .map(|answer| answer.unwrap_or_else(|error| serde_json::json!({ "error": error })))
    }
}

/// The cache slot one parameter row's answer lands in: the
/// parameter, and after a `#` the role it was asked by — the same
/// material's `eta` albedo box and its `eta` chromaticity box (and its
/// Metal colour, asked by any metal row) never overwrite each other.
/// A row without a role keeps the bare parameter's slot.
pub(super) fn swatch_slot(parameter: &str, role: Option<&str>) -> String {
    match role {
        Some(role) => format!("{parameter}#{role}"),
        None => parameter.to_owned(),
    }
}

/// What deselection leaves in place: a live scene node retires —
/// no marking anywhere, and the Inspector's page becomes the message
/// a pick that hits nothing shows — while a page that belongs to
/// something else (a Contents item, a value page) keeps its page,
/// because no node holds the selection.
fn deselect_state(
    selected_node: Option<String>,
    inspector: serde_json::Value,
) -> (Option<String>, serde_json::Value) {
    match selected_node {
        Some(_) => (
            None,
            serde_json::json!({"message": "Select a project node"}),
        ),
        None => (selected_node, inspector),
    }
}

/// Decoded preview cap: 128²×4B×64 ≈ 4MiB. Beyond it the least
/// recently used idle image is released and its entry re-asks when shown.
const MAX_PREVIEW_IMAGES: usize = 64;

/// A preview drawn this recently is on screen and never evicted: evicting
/// it would re-ask on the next render and evict another, forever. Node
/// editors only ask for cards on screen, but a zoomed-out graph (up to 256
/// nodes) can show more than the cap at once; the cap is soft then
/// (≤ 256 × 64KiB = 16MiB).
const PREVIEW_ACTIVE: Duration = Duration::from_secs(1);

/// B: the Inspector and node editors decode at 128px.
const INSPECTOR_PREVIEW_SIZE: u32 = 128;

impl Preview {
    /// Whether this entry should (re)ask for `project` now: a new
    /// revision, and no Busy backoff still pending.
    fn wants_request(&self, project: &serde_json::Value, now: Instant) -> bool {
        self.at != *project && self.retry_at.is_none_or(|at| at <= now)
    }

    /// A queued request answers this revision (a dropped one stays
    /// unasked, so the next render tries again), and clears any backoff.
    fn mark_requested(&mut self, project: &serde_json::Value) {
        self.at = project.clone();
        self.retry_at = None;
    }

    /// A Busy reply: ask again after the 250ms backoff, keeping the image.
    fn retry_busy(&mut self, now: Instant) {
        self.at = serde_json::Value::Null;
        self.retry_at = Some(now + Duration::from_millis(250));
    }

    /// A reply refused because the revision or activation moved on
    /// (StaleProject, RevisionConflict): not a failure of this preview, so
    /// the image stays on screen and the entry re-asks at the current one.
    fn superseded(&mut self) {
        self.at = serde_json::Value::Null;
    }

    /// A failed reply is a no-show for this revision: the old image is
    /// returned for release, `at` stays put.
    fn clear_image(&mut self) -> Option<Arc<RenderImage>> {
        self.image.take()
    }

    /// A decode that failed for `project`: the entry loses the image it
    /// holds for that same project (a no-show), while a failure belonging
    /// to a revision the entry moved on from touches nothing. Returns the
    /// released image.
    fn reject_image(&mut self, project: &serde_json::Value) -> Option<Arc<RenderImage>> {
        (self.at == *project).then(|| self.image.take()).flatten()
    }

    /// A decoded preview for `project` lands, replacing the old image;
    /// whatever returns (the replaced old one, or `image` itself for a
    /// revision the entry moved on from) is released by the caller.
    /// `key` is the reply's content key, remembered with the image.
    fn land_image(
        &mut self,
        project: &serde_json::Value,
        key: String,
        image: Arc<RenderImage>,
    ) -> Option<Arc<RenderImage>> {
        if self.at != *project {
            return Some(image);
        }
        self.key = Some(key);
        self.image.replace(image)
    }

    /// A reply whose content key matches the held image's at this
    /// revision needs neither a PNG read nor a decode — the image on
    /// screen is already the answer.
    fn keeps_image(&self, project: &serde_json::Value, key: &str) -> bool {
        self.at == *project && self.image.is_some() && self.key.as_deref() == Some(key)
    }
}

/// Keep at most [`MAX_PREVIEW_IMAGES`] decoded previews: the least
/// recently used idle entry (not drawn within [`PREVIEW_ACTIVE`]) loses its
/// image to the release list and re-asks when it is shown again (core's
/// cache answers quickly). On-screen previews are never evicted.
fn evict_preview_images(
    previews: &mut std::collections::HashMap<String, Preview>,
    drops: &mut Vec<Arc<RenderImage>>,
    now: Instant,
) {
    while previews
        .values()
        .filter(|entry| entry.image.is_some())
        .count()
        > MAX_PREVIEW_IMAGES
    {
        let Some(victim) = previews
            .iter()
            .filter(|(_, entry)| {
                entry.image.is_some()
                    && entry
                        .used
                        .is_none_or(|used| now.saturating_duration_since(used) >= PREVIEW_ACTIVE)
            })
            .min_by_key(|(_, entry)| entry.used)
            .map(|(id, _)| id.clone())
        else {
            return;
        };
        let Some(entry) = previews.get_mut(&victim) else {
            return;
        };
        if let Some(image) = entry.image.take() {
            drops.push(image);
        }
        entry.at = serde_json::Value::Null;
    }
}

/// Mean normalized RGB luminance of a decoded render, before channel reordering.
fn mean_luminance(rgba: &[u8]) -> Option<f64> {
    let (sum, count) = rgba
        .chunks_exact(4)
        .fold((0., 0usize), |(sum, count), pixel| {
            (
                sum + (0.2126 * f64::from(pixel[0])
                    + 0.7152 * f64::from(pixel[1])
                    + 0.0722 * f64::from(pixel[2]))
                    / 255.,
                count + 1,
            )
        });
    (count > 0).then_some(sum / count as f64)
}

/// A decoded preview PNG becomes a renderable image; Contents thumbnails
/// and Inspector previews share the BGRA swap.
fn render_image(mut rgba: image::RgbaImage) -> Arc<RenderImage> {
    for pixel in rgba.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]))
}

/// A same-activation revision keeps the decoded thumbnails; their
/// keys move to the new revision's project string and generation, the
/// same re-keying `ContentsThumbnails::revise` applied to its entries.
fn revise_thumbnail_images(
    images: &mut std::collections::HashMap<ThumbnailId, (String, Arc<RenderImage>)>,
    project: &str,
    generation: u64,
) {
    *images = images
        .drain()
        .map(|(mut id, held)| {
            id.project = project.to_owned();
            id.generation = generation;
            (id, held)
        })
        .collect();
}

fn decode_preview_png(path: &str, size: u32) -> Result<image::RgbaImage, String> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length == 0 || length > 2 * 1024 * 1024 {
        return Err("preview PNG exceeds byte limit".into());
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("preview PNG exceeds byte limit".into());
    }
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("invalid preview PNG".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width != size || height != size {
        return Err(format!("preview dimensions did not match {size}x{size}"));
    }
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    let image = image.to_rgba8();
    if image.dimensions() != (width, height) {
        return Err("decoded preview dimensions changed".into());
    }
    Ok(image)
}

pub(super) use super::parameter_values::ValuesPages;
impl Session {
    pub(super) fn remember_node_position(&mut self, root: &str, node: &str, position: (f32, f32)) {
        self.node_positions.remember(root, node, position);
        if self.project.mode == Some(protocol::project::ProjectMode::Project)
            && let Some(folder) = self.project.path.as_deref()
        {
            let path = super::graph_layout::position_file_path(std::path::Path::new(folder));
            if let Err(error) = self.node_positions.write(&path) {
                eprintln!("pbrt-ui: could not save node editor positions: {error}");
            }
        }
    }

    /// Classify the current document for menus, panels and the Inspector.
    pub(super) fn document(&self) -> super::document::DocumentState {
        super::document::DocumentState::from_state(&self.project)
    }

    /// The active project as wire JSON (`null` when empty). Panels still pass
    /// `ProjectRef` values through the untyped command seam.
    pub(super) fn project_value(&self) -> serde_json::Value {
        serde_json::to_value(self.project.context.project.clone())
            .unwrap_or(serde_json::Value::Null)
    }

    /// A node editor's value page names the owner's parameter; menus on
    /// its cards need the same Contents items the Inspector's rows use —
    /// the first owner page answered at this revision carries them (the
    /// backend attaches the whole project's items to every page).
    pub(super) fn current_items(&self) -> serde_json::Value {
        let project = self.project_value();
        self.resource_details
            .values()
            .find(|(read_at, _)| *read_at == project)
            .map(|(_, details)| details["items"].clone())
            .unwrap_or_default()
    }

    /// A Contents entry by id, from the Inspector's items or any node
    /// editor's details read at the current revision — links in a node
    /// editor must not depend on what the Inspector shows.
    pub(super) fn known_item(&self, id: &str) -> Option<serde_json::Value> {
        let project = self.project_value();
        std::iter::once(&self.project_inspector)
            .chain(
                self.resource_details
                    .values()
                    .filter(|(read_at, _)| *read_at == project)
                    .map(|(_, details)| details),
            )
            .map(|details| details["items"][id].clone())
            .find(serde_json::Value::is_object)
    }

    /// The decoded preview of a texture,
    /// material or image asset `id` at this revision, asking the backend
    /// for it once per revision (a Busy reply backs off 250ms and re-asks;
    /// anything else is a no-show until the revision moves). The last
    /// image stays on screen while a newer one is on its way.
    pub(super) fn preview(
        &mut self,
        id: &str,
        kind: PreviewKind,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        let project = self.project_value();
        if !project.is_object() {
            return None;
        }
        let now = Instant::now();
        let entry = self.previews.entry(id.to_owned()).or_default();
        if entry.wants_request(&project, now) {
            // Marked as asked only once the request is queued: a full
            // queue drops it, and the next render asks again.
            let queued = self
                .backend
                .commands
                .try_send(Command::Preview {
                    project: project.clone(),
                    id: id.to_owned(),
                    kind,
                })
                .is_ok();
            if queued {
                entry.mark_requested(&project);
            } else {
                cx.notify();
            }
        }
        entry.used = Some(now);
        let image = entry.image.clone();
        evict_preview_images(&mut self.previews, &mut self.pending_thumbnail_drops, now);
        image
    }

    pub(super) fn contents_visible(
        &mut self,
        project: serde_json::Value,
        list_generation: (u64, u64),
        candidates: Vec<(String, PreviewKind)>,
        cx: &mut Context<Self>,
    ) {
        if list_generation != (self.contents.generation(), self.contents_search_generation)
            || project != self.project_value()
        {
            return;
        }
        self.thumbnail_visible_at = Some(Instant::now());
        self.contents_thumbnails.set_visible(project, candidates);
        self.backend.update_thumbnail_visibility(
            self.project_value(),
            self.contents_thumbnails.generation(),
            self.contents_thumbnails.visible_snapshot(),
        );
        self.pump_contents_thumbnail(cx);
    }

    fn pump_contents_thumbnail(&mut self, cx: &mut Context<Self>) {
        if let Some(request_id) = self.thumbnail_done_pending.take()
            && self
                .backend
                .commands
                .try_send(Command::ThumbnailDone(request_id.clone()))
                .is_err()
        {
            self.thumbnail_done_pending = Some(request_id);
            return;
        }
        if self
            .thumbnail_visible_at
            .is_some_and(|at| at.elapsed() > Duration::from_millis(500))
        {
            self.contents_thumbnails
                .set_visible(self.project_value(), Vec::new());
            self.backend.update_thumbnail_visibility(
                self.project_value(),
                self.contents_thumbnails.generation(),
                Vec::new(),
            );
            self.thumbnail_visible_at = None;
        }
        if self.thumbnail_visible_at.is_none() {
            return;
        }
        if self.thumbnail_slot_active {
            return;
        }
        let Some(id) = self.contents_thumbnails.next_ready().into_iter().next() else {
            return;
        };
        let sent = self
            .backend
            .commands
            .try_send(Command::ThumbnailsVisible {
                project: self.project_value(),
                generation: id.generation,
                candidates: vec![(id.id.clone(), id.kind)],
            })
            .is_ok();
        if sent && self.contents_thumbnails.mark_running(&id) {
            self.thumbnail_slot_active = true;
            cx.notify();
        }
    }

    fn acknowledge_thumbnail(&mut self, request_id: String, cx: &mut Context<Self>) {
        if self
            .backend
            .commands
            .try_send(Command::ThumbnailDone(request_id.clone()))
            .is_err()
        {
            self.thumbnail_done_pending = Some(request_id);
        }
        self.pump_contents_thumbnail(cx);
    }

    /// Whether the Contents list still shows thumbnail `id`: in a loaded
    /// folder page, or among the current search hits (whose folders may
    /// never have been opened).
    fn contents_lists(&self, id: &ThumbnailId) -> bool {
        self.contents.contains_entry(id.kind, &id.id)
            || self.contents_search["entries"]
                .as_array()
                .is_some_and(|entries| {
                    entries
                        .iter()
                        .any(|entry| super::contents_tree::entry_is(entry, id.kind, &id.id))
                })
    }

    pub(super) fn thumbnail_image(
        &mut self,
        entry: &serde_json::Value,
    ) -> Option<Arc<RenderImage>> {
        let id = ThumbnailId {
            project: self.project_value().to_string(),
            id: entry["id"].as_str()?.to_owned(),
            kind: PreviewKind::of_entry(entry)?,
            generation: self.contents_thumbnails.generation(),
        };
        self.contents_thumbnails.touch(&id);
        self.thumbnail_images
            .get(&id)
            .map(|(_, image)| image.clone())
    }

    pub(super) fn clear_thumbnail_images(&mut self) {
        self.pending_thumbnail_drops
            .extend(self.thumbnail_images.drain().map(|(_, (_, image))| image));
    }

    fn drop_pending_thumbnail_images(&mut self, window: &mut Window) {
        for image in self.pending_thumbnail_drops.drain(..) {
            let _ = window.drop_image(image);
        }
    }

    pub(super) fn array_page_number(&self, form: &str) -> usize {
        self.values_page.get(form).copied().unwrap_or(0)
    }

    /// Show `page` of an open array row (the pager clicked).
    pub(super) fn array_show_page(&mut self, form: &str, page: usize, cx: &mut Context<Self>) {
        self.values_page.insert(form.to_owned(), page);
        cx.notify();
    }

    pub(super) fn array_reset_page(&mut self, form: &str, cx: &mut Context<Self>) {
        self.values_page.insert(form.to_owned(), 0);
        let project = self.project_value();
        if let Some(entry) = self.values_pages.get_mut(form) {
            entry.reset(project);
        }
        cx.notify();
    }

    /// The group page `form` shows and the core's payload for it, asking
    /// once per project: `None` until the answer lands (the table keeps
    /// a placeholder); the payload may hold `{error}`.
    pub(super) fn array_page(
        &mut self,
        form: &str,
        target: &serde_json::Value,
        name: &str,
        table: &super::parameter_values::Table,
        cx: &mut Context<Self>,
    ) -> Option<(usize, serde_json::Value)> {
        let project = self.project_value();
        if !project.is_object() {
            return None;
        }
        // A kept page can outlive the array it was picked for (apply,
        // undo, import can shrink it while the row is open): clamp it
        // into the current range so the window reads a real page, and
        // write it back so the pager and later clicks agree.
        let kept = self.values_page.get(form).copied().unwrap_or(0);
        let fresh = !self
            .values_pages
            .get(form)
            .is_some_and(|entry| entry.current(&project));
        let group_page = if fresh { 0 } else { table.clamped_page(kept) };
        if kept != group_page {
            self.values_page.insert(form.to_owned(), group_page);
        }
        let entry = self.values_pages.entry(form.to_owned()).or_default();
        if !entry.current(&project) {
            entry.reset(project.clone());
        }
        if !entry.asked(group_page) {
            let (offset, _) = table.window(group_page);
            let limit = super::parameter_values::PAGE_GROUPS * table.width;
            // Marked as asked only once the request is queued: a full
            // queue drops it, and the next render asks again.
            let queued = self
                .backend
                .commands
                .try_send(Command::GetParameterValues {
                    project: project.clone(),
                    target: target.clone(),
                    name: name.to_owned(),
                    offset,
                    limit,
                    form: form.to_owned(),
                    group_page,
                })
                .is_ok();
            if queued {
                entry.mark_asked(group_page);
            } else {
                cx.notify();
            }
        }
        entry
            .page(group_page)
            .cloned()
            .map(|page| (group_page, page))
    }

    /// One SpectrumData resource's whole sample table, asking the
    /// backend once per project: `None` until the answer lands; the
    /// payload may hold `{error}`.
    pub(super) fn spectrum_table(
        &mut self,
        resource_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<serde_json::Value> {
        let project = self.project_value();
        if !project.is_object() {
            return None;
        }
        let entry = self
            .spectrum_answers
            .entry(resource_id.to_owned())
            .or_default();
        if !entry.current(&project) {
            entry.reset(project.clone());
        }
        if !entry.table.asked() {
            // Marked as asked only once the request is queued: a full
            // queue drops it, and the next render asks again.
            let queued = self
                .backend
                .commands
                .try_send(Command::GetSpectrumTable {
                    project: project.clone(),
                    resource_id: resource_id.to_owned(),
                })
                .is_ok();
            if queued {
                entry.table = Asked::Pending;
            } else {
                cx.notify();
            }
        }
        entry
            .table
            .done()
            .cloned()
            .map(|table| table.unwrap_or_else(|error| serde_json::json!({ "error": error })))
    }

    /// One SpectrumData resource's approximate colour for `role`
    /// (`project.spectrum_color` behind the backend), asked once per
    /// project: `None` until the answer lands.
    pub(super) fn spectrum_swatch(
        &mut self,
        resource_id: &str,
        role: &str,
        cx: &mut Context<Self>,
    ) -> Option<serde_json::Value> {
        let project = self.project_value();
        if !project.is_object() {
            return None;
        }
        let entry = self
            .spectrum_answers
            .entry(resource_id.to_owned())
            .or_default();
        if !entry.current(&project) {
            entry.reset(project.clone());
        }
        let asked = entry.swatches.get(role).is_some_and(Asked::asked);
        if !asked {
            let queued = self
                .backend
                .commands
                .try_send(Command::SpectrumSwatch {
                    project: project.clone(),
                    resource_id: resource_id.to_owned(),
                    role: role.to_owned(),
                })
                .is_ok();
            if queued {
                entry.swatches.insert(role.to_owned(), Asked::Pending);
            } else {
                cx.notify();
            }
        }
        entry
            .swatches
            .get(role)?
            .done()
            .cloned()
            .map(|answer| answer.unwrap_or_else(|error| serde_json::json!({ "error": error })))
    }

    /// One parameter row's approximate colour
    /// (`project.spectrum_color` behind the backend, for the row's role —
    /// None lets the core derive it from the schema; a Metal colour row
    /// asks the conductor role), asked at most once per visible row per
    /// revision: `None` until the answer lands; the payload may hold
    /// `{error}`. The query itself runs on the display thread — this
    /// only queues the command and returns, so the row's render never
    /// waits on the wire. The cache slot keys `parameter#role` so the
    /// same resource keeps its albedo box and its Metal colour apart.
    pub(super) fn parameter_swatch(
        &mut self,
        target: &serde_json::Value,
        parameter: &str,
        role: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<serde_json::Value> {
        let project = self.project_value();
        if !project.is_object() {
            return None;
        }
        let id = target["id"].as_str()?.to_owned();
        let slot = swatch_slot(parameter, role);
        let entry = self.parameter_answers.entry(id).or_default();
        if entry.asking(&project, &slot) {
            // Marked as asked only once the request is queued: a full
            // queue drops it, and the next render asks again.
            let queued = self
                .backend
                .commands
                .try_send(Command::ParameterSpectrumColour {
                    project: project.clone(),
                    target: target.clone(),
                    parameter: parameter.to_owned(),
                    role: role.map(str::to_owned),
                })
                .is_ok();
            if queued {
                entry.mark_pending(&slot);
            } else {
                cx.notify();
            }
        }
        entry.swatch_answer(&slot)
    }

    /// Where item `(kind, id)` is used, asking the backend once per
    /// project. Returns None until the answer lands (the first render
    /// opens the list empty and the next one fills it).
    pub(super) fn item_references(
        &mut self,
        kind: &str,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Option<serde_json::Value> {
        let project = self.project_value();
        if !project.is_object() {
            return None;
        }
        let asked = format!("{kind}:{id}");
        let known = self
            .item_references
            .get(&asked)
            .is_some_and(|(at, _)| *at == project);
        if !known {
            let queued = self
                .backend
                .commands
                .try_send(Command::ListReferences {
                    project: project.clone(),
                    item: (kind.to_owned(), id.to_owned()),
                })
                .is_ok();
            if queued {
                // Re-tag a stale entry too: `or_insert_with` alone would
                // keep the previous revision's tag, every later
                // Update::ListReferences would arrive as not-current and
                // be dropped, and the queue would spin forever.
                self.item_references
                    .entry(asked.clone())
                    .and_modify(|entry| *entry = (project.clone(), serde_json::Value::Null))
                    .or_insert_with(|| (project.clone(), serde_json::Value::Null));
            } else {
                cx.notify();
            }
        }
        self.item_references
            .get(&asked)
            .map(|(_, list)| list.clone())
    }

    /// The current activation guard used by open/import/close.
    pub(super) fn context_value(&self) -> serde_json::Value {
        serde_json::to_value(&self.project.context).unwrap_or(serde_json::Value::Null)
    }

    /// Select a Contents entry: the Hierarchy selection clears and the
    /// Inspector shows the entry. A node editor's value page gives way
    /// — the Inspector moved.
    pub(super) fn select_contents(
        &mut self,
        entry: serde_json::Value,
        project: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        self.contents.select_entry(&entry);
        self.selected_project_node = None;
        self.graph_value = None;
        self.inspect_asked = Some(entry.clone());
        self.send(Command::InspectContents { project, entry }, cx);
    }

    /// Retire the selected scene node. The Inspector takes the page
    /// a pick that hits nothing shows, the Hierarchy row highlight and
    /// the gizmo follow the marking, and the backend's own retiral
    /// keeps its per-revision refresh from re-emitting the node's page
    /// and pulling the Inspector straight back onto the node. Nothing
    /// deselects when no node holds the selection: a Contents item or
    /// a value page keeps its page. Selection is not an edit — the
    /// Undo history stays put.
    pub(super) fn deselect_node(&mut self, cx: &mut Context<Self>) {
        if self.selected_project_node.is_none() {
            return;
        }
        let (node, page) = deselect_state(
            self.selected_project_node.take(),
            std::mem::take(&mut self.project_inspector),
        );
        self.selected_project_node = node;
        self.project_inspector = page;
        self.send(Command::ProjectDeselect, cx);
    }

    /// The node editor selects a value node: the Inspector shows the
    /// inline colour page of its owner's parameter until another
    /// selection moves it, and the node editor highlights the node.
    /// A scene node selected before — a Hierarchy or a viewport pick —
    /// retires at both ends: the session's marking (the Hierarchy row
    /// follows), and the backend's selection, whose per-reference
    /// refresh would re-emit the node's inspector on the next revision
    /// and the `ProjectInspector` handling would move the Inspector
    /// off this very page.
    pub(super) fn set_graph_value(
        &mut self,
        root: String,
        owner: String,
        parameter: String,
        cx: &mut Context<Self>,
    ) {
        let target = (root, owner, parameter);
        if self
            .graph_value
            .as_ref()
            .is_some_and(|current| *current == target)
        {
            return;
        }
        if super::graph_selection::value_selection_retires_node(
            self.selected_project_node.as_deref(),
        ) {
            self.selected_project_node = None;
            self.send(Command::ProjectDeselect, cx);
        }
        self.graph_value = Some(target);
        cx.notify();
    }

    /// The node editor's selected value node, as the Inspector and the
    /// highlight read it: (graph root, owner resource id, parameter).
    pub(super) fn graph_value(&self) -> Option<(&str, &str, &str)> {
        self.graph_value
            .as_ref()
            .map(|(root, owner, parameter)| (root.as_str(), owner.as_str(), parameter.as_str()))
    }

    /// Whether the colour dialog of `form` is open — the
    /// dialog, not the edit: a field rebuilt while the user had it open
    /// renders its popover open from this, because the popover's own
    /// state dies with the field. Only the popover's announced dismissal
    /// clears it.
    pub(super) fn colour_dialog_open(&self, form: &str) -> bool {
        self.colour_dialogs.is_open(form)
    }

    /// The popover of `form` reports its own open state:
    /// every announced open marks it, every announced close clears it.
    pub(super) fn set_colour_dialog_open(&mut self, form: &str, open: bool) {
        self.colour_dialogs.set_open(form, open);
    }

    /// The node editor that is about to apply a background
    /// creation, with the graph-units point its menu picked; the key
    /// rides the apply's follow, so its answer (or failure) settles
    /// exactly this ask.
    pub(super) fn await_node_create(
        &mut self,
        editor: WeakEntity<super::node_editor::NodeEditor>,
        point: Option<(f32, f32)>,
    ) -> u64 {
        self.node_editor_create_key += 1;
        let key = self.node_editor_create_key;
        self.node_editor_creates
            .insert(key, super::node_editor::CreateAsk { editor, point });
        key
    }

    pub(super) fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        let render_request = matches!(&command, Command::Render(_));
        if matches!(
            command,
            Command::Mode(_)
                | Command::Time(_)
                | Command::FrameAll
                | Command::FrameSelection
                | Command::Orbit(..)
                | Command::Pan(..)
                | Command::Fly(..)
                | Command::Dolly(_)
        ) {
            self.updating = true;
        }
        let gizmo = matches!(command, Command::GizmoPreview(_))
            || command.form() == Some(super::GIZMO_FORM);
        let resets = gizmo && !matches!(command, Command::GizmoPreview(Some(_)));
        let sent = self.backend.commands.try_send(command);
        if gizmo && sent.is_ok() {
            // Any preview sets the offset outright: it supersedes a pending reset.
            self.gizmo_reset = false;
        }
        if let Err(error) = sent {
            if render_request {
                self.render_start.request_failed();
            }
            // Left unsent, the backend would keep showing an uncommitted move.
            self.gizmo_reset |= resets;
            self.status = format!("Display queue: {error}");
            self.updating = false;
            // The backend never saw it: settle the form that sent it, which
            // would otherwise wait for an answer that cannot come.
            let command = match error {
                std::sync::mpsc::TrySendError::Full(command)
                | std::sync::mpsc::TrySendError::Disconnected(command) => command,
            };
            if let Some(form) = command.form() {
                self.edit_feedback
                    .rejected(form, "The display is busy; try again");
            }
        }
        cx.notify();
    }

    pub(super) fn start_render(&mut self, current_view: bool, cx: &mut Context<Self>) {
        if self.render_start.active() {
            return;
        }
        self.render_start.begin(&self.jobs, Instant::now());
        cx.notify();
        self.send(Command::Render(current_view), cx);
    }

    pub(super) fn request_contents_search(
        &mut self,
        project: serde_json::Value,
        query: String,
        generation: u64,
        offset: usize,
        limit: usize,
        cx: &mut Context<Self>,
    ) {
        if !super::project_contents_search::request_is_current(
            &project,
            &query,
            generation,
            &self.contents_search_project,
            &self.contents_search_query,
            self.contents_search_generation,
        ) || self.project_value() != project
        {
            return;
        }
        self.contents_search_in_flight = Some((generation, offset));
        self.contents_search_settled = None;
        self.contents_search_error = None;
        if self
            .backend
            .commands
            .try_send(Command::ProjectSearchContents {
                project,
                query,
                generation,
                offset,
                limit,
            })
            .is_err()
        {
            self.contents_search_in_flight = None;
            self.contents_search_settled = Some((generation, offset));
            self.contents_search_error = Some("Display queue is busy; retry search".into());
        }
        cx.notify();
    }
    pub(super) fn new(backend: Backend, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let time_slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(60.)
                .step(1.)
                .default_value(30.)
        });
        cx.subscribe(&time_slider, |this, _, event, cx| {
            if let SliderEvent::Change(value) = event {
                let index = value.start().round().max(0.) as usize;
                if let Some(time) = this.supported_times.get(index).copied() {
                    this.view_time = time;
                    this.send(Command::Time(time), cx);
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let mut changed = false;
                        this.drop_pending_thumbnail_images(window);
                        this.pump_contents_thumbnail(cx);
                        // A Busy-backed-off preview whose 250ms elapsed: the
                        // notify re-renders the panels still showing it, and
                        // re-rendering re-asks.
                        // Consuming the deadline keeps a preview that stopped
                        // rendering from repainting at 33ms forever.
                        let mut preview_retry_due = false;
                        for entry in this.previews.values_mut() {
                            if entry.retry_at.is_some_and(|at| at <= Instant::now()) {
                                entry.retry_at = None;
                                preview_retry_due = true;
                            }
                        }
                        changed |= preview_retry_due;
                        if this.gizmo_reset
                            && this
                                .backend
                                .commands
                                .try_send(Command::GizmoPreview(None))
                                .is_ok()
                        {
                            this.gizmo_reset = false;
                        }
                        while let Ok(update) = this.backend.updates.try_recv() {
                            changed = true;
                            match update {
                                Update::LayoutReset => this.layout_revision += 1,
                                Update::Quit => cx.quit(),
                                Update::Jobs(jobs) => {
                                    this.render_start.observe_jobs(&jobs, Instant::now());
                                    this.jobs = jobs;
                                }
                                Update::ForegroundJobs(jobs) => this.foreground_jobs = jobs,
                                Update::Settings(settings) => this.settings = settings,
                                Update::Ipr(status) => this.ipr = status,
                                Update::RenderImage {
                                    width,
                                    height,
                                    mut rgba,
                                    artifact_id,
                                    project,
                                    received,
                                } => {
                                    this.render_artifact = artifact_id;
                                    this.render_project = project;
                                    this.render_received = received;
                                    this.render_mean_luminance = mean_luminance(&rgba);
                                    for pixel in rgba.chunks_exact_mut(4) {
                                        pixel.swap(0, 2);
                                    }
                                    if let Some(buffer) =
                                        image::RgbaImage::from_raw(width, height, rgba)
                                    {
                                        let image =
                                            Arc::new(RenderImage::new(vec![image::Frame::new(
                                                buffer,
                                            )]));
                                        if let Some(old) = this.render_image.replace(image) {
                                            let _ = window.drop_image(old);
                                        }
                                    }
                                }
                                Update::Mode(mode) => this.preview_mode = mode,
                                Update::Time(time) => {
                                    this.view_time = time;
                                    if let Some(index) = this
                                        .supported_times
                                        .iter()
                                        .position(|sample| *sample == time)
                                    {
                                        this.time_slider.update(cx, |slider, cx| {
                                            slider.set_value(index as f32, window, cx)
                                        });
                                    }
                                }
                                Update::Status(status) => {
                                    if status.starts_with("Preview:") {
                                        this.updating = false;
                                    }
                                    record_status(&mut this.status, &mut this.messages, status);
                                }
                                Update::RenderRequestFailed => {
                                    this.render_start.request_failed();
                                }
                                Update::Project(project) => {
                                    let old = &this.project.context.project;
                                    let new = &project.context.project;
                                    let activation_changed = match (old, new) {
                                        (Some(a), Some(b)) => {
                                            a.project_id != b.project_id
                                                || a.activation != b.activation
                                        }
                                        (None, None) => false,
                                        _ => true,
                                    };
                                    if activation_changed {
                                        // The tree is not tagged with a project reference on
                                        // the update channel. Drop the previous activation's
                                        // placement targets until its replacement arrives.
                                        clear_project_tree_on_activation_change(
                                            &mut this.project_tree,
                                            true,
                                        );
                                        // A where-used reveal must outlive the activation
                                        // bump its own project.open_scene causes,
                                        // so only a project replacement or close drops
                                        // it here.
                                        let project_left = match (old, new) {
                                            (Some(a), Some(b)) => a.project_id != b.project_id,
                                            _ => true,
                                        };
                                        if project_left {
                                            this.reveal_where_used = None;
                                        }
                                        if let Some(image) = this.render_image.take() {
                                            let _ = window.drop_image(image);
                                        }
                                        this.render_artifact = None;
                                        this.render_project = serde_json::Value::Null;
                                        this.render_mean_luminance = None;
                                        this.render_received = None;
                                        this.selected_project_node = None;
                                        this.project_inspector = serde_json::Value::Null;
                                        this.graph_value = None;
                                        this.project_search_query.clear();
                                        this.project_search = serde_json::Value::Null;
                                        this.contents_search_query.clear();
                                        this.contents_search = serde_json::Value::Null;
                                        this.contents_search_project = serde_json::Value::Null;
                                        this.contents_search_generation = this.contents_search_generation.wrapping_add(1);
                                        this.contents_search_error = None;
                                        this.contents_search_in_flight = None;
                                        this.contents_search_settled = None;
                                        this.contents.reset();
                                        this.resource_graphs.clear();
                                        this.resource_details.clear();
                                        this.item_references.clear();
                                        // Preview images outlive their entries;
                                        // hand them to the release list
                                        // instead of dropping them in place.
                                        this.pending_thumbnail_drops.extend(
                                            this.previews
                                                .drain()
                                                .filter_map(|(_, entry)| entry.image),
                                        );
                                        this.project_scenes = serde_json::Value::Null;
                                        this.reveal_created = None;
                                        this.reveal_picked = None;
                                        // U-f): the awaits belong to
                                        // the document that asked; a creation
                                        // answered across the switch lands
                                        // nowhere.
                                        this.node_editor_creates.clear();
                                        this.import_warnings = 0;
                                        this.unsaved_changes = false;
                                    }
                                    if this.project.context.project != project.context.project
                                        || this.project.active_scene != project.active_scene
                                    {
                                        let same_scene = same_preview_owner(
                                            this.project.context.project.as_ref(),
                                            this.project.active_scene,
                                            project.context.project.as_ref(),
                                            project.active_scene,
                                        );
                                        if same_scene {
                                            this.stale_frame = true;
                                        } else {
                                            if let Some(image) = this.image.take() {
                                                let _ = window.drop_image(image);
                                            }
                                            this.stale_frame = false;
                                        }
                                        this.preview_error = None;
                                        this.updating = project.active_scene.is_some();
                                        this.scene_key = format!(
                                            "project-loading:{}",
                                            serde_json::json!([
                                                &project.context.project,
                                                &project.active_scene
                                            ])
                                        );
                                    }
                                    let revision_changed =
                                        this.project.context.project != project.context.project;
                                    this.project = project;
                                    // The folder can arrive after the activation
                                    // changes, so follow the position file itself.
                                    let file = (this.project.mode
                                        == Some(protocol::project::ProjectMode::Project))
                                    .then(|| this.project.path.as_deref())
                                    .flatten()
                                    .map(std::path::Path::new)
                                    .map(super::graph_layout::position_file_path);
                                    if file != this.node_positions_file {
                                        let loaded = file
                                            .as_deref()
                                            .map(super::graph_layout::PositionStore::read)
                                            .unwrap_or_default();
                                        // A Viewer saved as a project keeps its layout.
                                        if this.node_positions_file.is_some()
                                            || loaded != Default::default()
                                        {
                                            this.node_positions = loaded;
                                            this.node_positions_loaded += 1;
                                        }
                                        this.node_positions_file = file;
                                    }
                                    if revision_changed && !activation_changed {
                                        // The search box survives the revision.
                                        // The same query is re-run at the new revision
                                        // Its current page stays
                                        // on screen until the fresh one lands.
                                        let current = this.project_value();
                                        let query = this.contents_search_query.clone();
                                        if !query.is_empty() && current.is_object() {
                                            this.contents_search_generation =
                                                this.contents_search_generation.wrapping_add(1);
                                            this.contents_search_project = current.clone();
                                            this.contents_search_error = None;
                                            this.contents_search_in_flight = None;
                                            this.contents_search_settled = None;
                                            this.request_contents_search(
                                                current,
                                                query,
                                                this.contents_search_generation,
                                                0,
                                                super::project_contents_search::SEARCH_PAGE_LIMIT,
                                                cx,
                                            );
                                        }
                                    }
                                    if activation_changed {
                                        let current = this.project_value();
                                        this.contents_thumbnails.invalidate(current);
                                        this.backend.update_thumbnail_visibility(
                                            this.project_value(), this.contents_thumbnails.generation(), Vec::new(),
                                        );
                                        this.clear_thumbnail_images();
                                    } else if revision_changed {
                                        // A same-activation revision keeps
                                        // every thumbnail — entries re-key to the new
                                        // revision and re-ask to confirm their content
                                        // key; a matching key re-enters Ready without
                                        // reading the PNG again, so the panel never
                                        // blanks while it re-checks.
                                        let current = this.project_value();
                                        let project_ref = current.to_string();
                                        this.contents_thumbnails.revise(current);
                                        revise_thumbnail_images(
                                            &mut this.thumbnail_images,
                                            &project_ref,
                                            this.contents_thumbnails.generation(),
                                        );
                                    }
                                    // Expanded Contents folders follow the revision.
                                    let current = this.project_value();
                                    // So does the Inspector of a Contents item (the
                                    // backend re-reads a Hierarchy selection itself):
                                    // a resource's implementation or parameters may
                                    // have changed. Refresh the item the
                                    // Inspector shows — the Contents selection can
                                    // point elsewhere.
                                    // A node editor's value page keeps its place:
                                    // the page refreshes through the node editor's
                                    // own details ask, and this re-ask would only
                                    // pull the Inspector target back out.
                                    // An ask still in flight names the item the
                                    // Inspector is about to show, not the old page's.
                                    let item = this
                                        .inspect_asked
                                        .as_ref()
                                        .unwrap_or(&this.project_inspector["item"]);
                                    if revision_changed
                                        && !activation_changed
                                        && current.is_object()
                                        && item.is_object()
                                        && this.graph_value.is_none()
                                    {
                                        let _ = this.backend.commands.try_send(
                                            Command::InspectContents {
                                                project: current.clone(),
                                                entry: item.clone(),
                                            },
                                        );
                                    }
                                    if current.is_object() {
                                        for folder in this.contents.stale_folders(&current) {
                                            let _ = this.backend.commands.try_send(
                                                Command::ProjectContents {
                                                    project: current.clone(),
                                                    folder,
                                                    offset: 0,
                                                    limit: super::contents_tree::PAGE_LIMIT,
                                                },
                                            );
                                        }
                                    }
                                }
                                Update::ProjectTree(tree) => {
                                    let mut tree = tree;
                                    let scope = match &this.project.context.project {
                                        Some(reference) => format!(
                                            "{}:{}",
                                            reference.project_id,
                                            reference.activation.as_str()
                                        ),
                                        None => String::new(),
                                    };
                                    // Same activation, new revision: keep the loaded
                                    // branches on screen and mark them for refresh.
                                    if !scope.is_empty() && scope == this.tree_scope {
                                        crate::project_tree::retain_loaded(
                                            &mut tree,
                                            &this.project_tree,
                                        );
                                    }
                                    this.tree_scope = scope;
                                    this.project_tree = tree;
                                }
                                Update::ProjectSearch {
                                    query,
                                    offset,
                                    page,
                                } => {
                                    let current = this.project_value();
                                    if page.is_null() {
                                        // A failed page invalidates the accumulated
                                        // result; the status line explains why.
                                        if offset == 0 || this.project_search_query == query {
                                            this.project_search_query.clear();
                                            this.project_search = serde_json::Value::Null;
                                        }
                                    } else if page["project"] == current {
                                        // A new query starts over even if its first
                                        // page arrives with a nonzero offset.
                                        if this.project_search_query != query {
                                            this.project_search = serde_json::Value::Null;
                                        }
                                        crate::project_search::merge_page(
                                            &mut this.project_search,
                                            offset,
                                            &page,
                                        );
                                        this.project_search_query = query;
                                    }
                                }
                                Update::ProjectSearchContents {
                                    project,
                                    query,
                                    generation,
                                    offset,
                                    page,
                                    error,
                                } => {
                                    if !super::project_contents_search::request_is_current(
                                        &project,
                                        &query,
                                        generation,
                                        &this.contents_search_project,
                                        &this.contents_search_query,
                                        this.contents_search_generation,
                                    ) || this.project_value() != project
                                    {
                                        return;
                                    }
                                    if page.is_null() {
                                        super::project_contents_search::apply_page(
                                            &mut this.contents_search,
                                            offset,
                                            &page,
                                        );
                                        this.contents_search_error = error;
                                    } else if super::project_contents_search::response_is_current(
                                        &project,
                                        &query,
                                        generation,
                                        &this.contents_search_project,
                                        &this.contents_search_query,
                                        this.contents_search_generation,
                                        &page,
                                    ) {
                                        if super::project_contents_search::apply_page(
                                            &mut this.contents_search,
                                            offset,
                                            &page,
                                        ) {
                                            this.contents_search_error = None;
                                        }
                                    }
                                    this.contents_search_in_flight = None;
                                    this.contents_search_settled = Some((generation, offset));
                                }
                                Update::ProjectChildren {
                                    parent,
                                    offset,
                                    nodes,
                                    next_offset,
                                } => {
                                    // A tree snapshot of the same revision can mark the
                                    // branch stale again after this page; the request is
                                    // answered either way.
                                    this.children_arrived.push(parent.clone());
                                    let mut tree = this.project_tree.clone();
                                    if crate::project_preview::merge_project_children(
                                        &mut tree,
                                        &parent,
                                        offset,
                                        &nodes,
                                        next_offset,
                                    )
                                    .is_some()
                                    {
                                        this.project_tree = tree;
                                    }
                                }
                                Update::ProjectContents {
                                    project,
                                    folder,
                                    offset,
                                    page,
                                } => {
                                    if page.is_null() {
                                        // The folder was moved or deleted; the
                                        // status line carries the reason.
                                        this.contents.drop_folder(&folder);
                                    } else if project == this.project_value() {
                                        this.contents.merge_page(&project, &folder, offset, &page);
                                    }
                                }
                                Update::ProjectScenes(scenes) => this.project_scenes = scenes,
                                Update::ProjectPresets(presets) => this.project_presets = presets,
                                Update::ResourceSchemas(summary) => this.resource_schema = summary,
                                Update::EditorCreateFailed(ask) => {
                                    this.node_editor_creates.remove(&ask);
                                }
                                // Contents ▸ New: reveal the
                                // created resource, select it, enter rename.
                                // An editor-requested creation
                                // opens the Inspector and hands the id to
                                // the editor that asked, which places the
                                // node under its click — no rename prompt.
                                Update::CreatedResource {
                                    project,
                                    entry,
                                    editor_ask,
                                } => {
                                    // Settled whatever happens below.
                                    let ask = editor_ask
                                        .map(|key| this.node_editor_creates.remove(&key));
                                    let current = this.project_value();
                                    // Same project and activation; only the revision
                                    // may be newer than the window's.
                                    let same = |key: &str| project[key] == current[key];
                                    if current.is_object()
                                        && same("project_id")
                                        && same("activation")
                                    {
                                        let location = entry["location"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_owned();
                                        for folder in this.contents.reveal(&location, &current) {
                                            let _ = this.backend.commands.try_send(
                                                Command::ProjectContents {
                                                    project: current.clone(),
                                                    folder,
                                                    offset: 0,
                                                    limit: super::contents_tree::PAGE_LIMIT,
                                                },
                                            );
                                        }
                                        // Inspect at the revision that holds it;
                                        // the hand-off reads the id from the entry.
                                        let created_id =
                                            entry["id"].as_str().map(str::to_owned);
                                        this.select_contents(entry, project, cx);
                                        if let Some(ask) = ask {
                                            window.dispatch_action(
                                                Box::new(super::menu::ShowInspector),
                                                cx,
                                            );
                                            if let (Some(ask), Some(id)) = (ask, created_id) {
                                                // Deferred: the editor reads this
                                                // session, which is mid-update here.
                                                cx.defer(move |cx| {
                                                    let _ = ask.editor.update(cx, |editor, cx| {
                                                        editor.node_created(id, ask.point, cx)
                                                    });
                                                });
                                            }
                                        } else {
                                            window.dispatch_action(
                                                Box::new(super::menu::ContentsRename),
                                                cx,
                                            );
                                        }
                                    }
                                }
                                Update::Created {
                                    node,
                                    wrapper,
                                    parent,
                                } => this.reveal_created = Some((node, wrapper, parent)),
                                Update::RevealPicked(node, path) => {
                                    this.reveal_picked = Some((node, path))
                                }
                                Update::ProjectInspector(inspector) => {
                                    // One selection at a time: a node clears the
                                    // Contents selection and an item the node;
                                    // any Inspector target that arrives here —
                                    // a Contents click, a Hierarchy pick —
                                    // moves a node editor's value page away.
                                    if inspector["node"].is_object() {
                                        this.contents.select(None);
                                    }
                                    this.graph_value = None;
                                    this.inspect_asked = None;
                                    // Another selection: earlier field
                                    // messages belong to the previous one.
                                    let subject = |details: &serde_json::Value| {
                                        (
                                            details["node"]["node_id"].clone(),
                                            details["item"]["id"].clone(),
                                            details["item"]["location"].clone(),
                                        )
                                    };
                                    if subject(&inspector) != subject(&this.project_inspector) {
                                        this.edit_feedback.clear();
                                    }
                                    this.selected_project_node =
                                        inspector["node"]["node_id"].as_str().map(str::to_owned);
                                    this.project_inspector = inspector;
                                }
                                Update::ProjectDeselected => {
                                    // An Inspector answer queued before the
                                    // deselect may have put the node back.
                                    let (node, page) = deselect_state(
                                        this.selected_project_node.take(),
                                        std::mem::take(&mut this.project_inspector),
                                    );
                                    this.selected_project_node = node;
                                    this.project_inspector = page;
                                }
                                Update::RenderCamera(label) => {
                                    this.render_camera_label = label;
                                }
                                Update::PreviewUnavailable(reason) => {
                                    this.updating = false;
                                    this.stale_frame = this.image.is_some();
                                    this.preview_error = reason;
                                }
                                Update::ProjectPreview { key, times, time } => {
                                    this.scene_key = key;
                                    this.supported_times =
                                        if times.is_empty() { vec![0.] } else { times };
                                    this.view_time = time;
                                    let time_index = this
                                        .supported_times
                                        .iter()
                                        .position(|t| *t == time)
                                        .unwrap_or(0);
                                    this.time_slider.update(cx, |slider, cx| {
                                        slider.set_value(time_index as f32, window, cx)
                                    });
                                    this.updating = true;
                                    this.stale_frame = this.image.is_some();
                                    this.preview_error = None;
                                }
                                Update::Log(message) => {
                                    this.messages.push(message);
                                    if this.messages.len() > 200 {
                                        this.messages.remove(0);
                                    }
                                }
                                Update::SaveSettled { ok, error } => {
                                    this.save_in_flight = false;
                                    this.save_settled = Some((ok, error));
                                }
                                Update::UnsavedChanges => this.unsaved_changes = true,
                                Update::EditorReady { project, request_id, entry, path } => {
                                    this.editors_ready.push((project, request_id, entry, path))
                                }
                                Update::ItemsMissing { project, ids } => {
                                    this.items_missing.push((project, ids));
                                }
                                // A change that removes or keeps unknown
                                // parameters asks first; the plan's edits go
                                // out as one transaction.
                                Update::ImplementationPlan {
                                    project,
                                    form,
                                    plan,
                                } => {
                                    let edits =
                                        plan["edits"].as_array().cloned().unwrap_or_default();
                                    if project == this.project_value() && !edits.is_empty() {
                                        let apply = move |session: &mut Session,
                                                          cx: &mut Context<Session>| {
                                            session.edit_feedback.submitted(&form);
                                            session.send(
                                                Command::Apply {
                                                    project,
                                                    edits,
                                                    follow: backend::Follow::Nothing,
                                                    form: Some(form),
                                                },
                                                cx,
                                            );
                                        };
                                        match super::implementation_menu::confirmation(&plan) {
                                            None => apply(this, cx),
                                            Some(description) => {
                                                let session = cx.entity();
                                                let apply = std::cell::Cell::new(Some(apply));
                                                super::prompt::confirm(
                                                    window,
                                                    cx,
                                                    format!(
                                                        "Change to {}?",
                                                        plan["implementation"]
                                                            .as_str()
                                                            .unwrap_or_default()
                                                    ),
                                                    description,
                                                    "Change",
                                                    move |_, cx| {
                                                        if let Some(apply) = apply.take() {
                                                            session.update(cx, |session, cx| {
                                                                apply(session, cx)
                                                            });
                                                        }
                                                    },
                                                );
                                            }
                                        }
                                    }
                                }
                                Update::EditSettled { form, error } => match error {
                                    Some(error) => this.edit_feedback.rejected(&form, &error),
                                    None => this.edit_feedback.applied(&form),
                                },
                                Update::ImportWarnings(count) => this.import_warnings = count,
                                // Node editors read these on render.
                                Update::ResourceGraph {
                                    project,
                                    resource_id,
                                    graph,
                                } => {
                                    this.resource_graphs.insert(resource_id, (project, graph));
                                }
                                Update::ListReferences {
                                    project,
                                    item,
                                    list,
                                } => {
                                    // Only the current project's answer lands;
                                    // anything else stays a no-show.
                                    let asked = format!("{}:{}", item.0, item.1);
                                    if let Some(entry) = this.item_references.get_mut(&asked) {
                                        if entry.0 == project {
                                            entry.1 = list;
                                            cx.notify();
                                        }
                                    }
                                }
                                Update::Preview { project, id, result } => {
                                    // A reply the entry no longer wants
                                    // (moved on, evicted, gone with the
                                    // activation) is dropped whole.
                                    if let Some(entry) = this.previews.get_mut(&id)
                                        && entry.at == project
                                    {
                                        match result {
                                            // Core's workers are saturated:
                                            // ask again after the backoff,
                                            // keeping the current image.
                                            Err(error)
                                                if error.code
                                                    == protocol::ErrorCode::Busy =>
                                            {
                                                entry.retry_busy(Instant::now());
                                            }
                                            // The revision moved on before
                                            // core answered: keep the image
                                            // and re-ask.
                                            Err(error)
                                                if matches!(
                                                    error.code,
                                                    protocol::ErrorCode::StaleProject
                                                        | protocol::ErrorCode::RevisionConflict
                                                ) =>
                                            {
                                                entry.superseded();
                                            }
                                            // Any other failure is a no-show
                                            // for this revision.
                                            Err(_) => {
                                                if let Some(old) = entry.clear_image() {
                                                    this.pending_thumbnail_drops.push(old);
                                                }
                                            }
                                            Ok((path, key)) => {
                                                // A reply whose content
                                                // key matches the held image
                                                // at this revision is this
                                                // revision's answer — the
                                                // PNG is neither read nor
                                                // decoded again.
                                                if !entry.keeps_image(&project, &key) {
                                                    // Decode off the UI thread;
                                                    // the entry may have moved on
                                                    // by the time the image lands.
                                                    let entity = cx.entity();
                                                    let decoder = cx
                                                        .background_executor()
                                                        .spawn(async move {
                                                            decode_preview_png(
                                                                &path,
                                                                INSPECTOR_PREVIEW_SIZE,
                                                            )
                                                        });
                                                    cx.spawn(async move |_, cx| {
                                                        let decoded = decoder.await;
                                                        entity.update(cx, |session, cx| {
                                                            if let Some(entry) = session
                                                                .previews
                                                                .get_mut(&id)
                                                            {
                                                                match decoded {
                                                                    Ok(rgba) => {
                                                                        let image =
                                                                            render_image(rgba);
                                                                        if let Some(old) = entry
                                                                            .land_image(
                                                                                &project,
                                                                                key,
                                                                                image,
                                                                            )
                                                                        {
                                                                            session
                                                                                .pending_thumbnail_drops
                                                                                .push(old);
                                                                        }
                                                                    }
                                                                    Err(_) => {
                                                                        if let Some(old) = entry
                                                                            .reject_image(&project)
                                                                        {
                                                                            session
                                                                                .pending_thumbnail_drops
                                                                                .push(old);
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            cx.notify();
                                                        });
                                                    })
                                                    .detach();
                                                }
                                            }
                                        }
                                    }
                                }
                                Update::Thumbnail { project, id, kind, generation, request_id, result } => {
                                    let id = ThumbnailId { project: project.to_string(), id, kind, generation };
                                    match result {
                                        Err(error) => {
                                            if error.code == protocol::ErrorCode::Busy { this.contents_thumbnails.busy(&id); }
                                            else if error.code == protocol::ErrorCode::Cancelled { this.contents_thumbnails.cancelled_before_start(&id); }
                                            else { this.contents_thumbnails.failed(&id); }
                                            this.thumbnail_slot_active = false;
                                            this.acknowledge_thumbnail(request_id, cx);
                                            cx.notify();
                                        }
                                        Ok((path, key)) if project == this.project_value()
                                            && this.contents_lists(&id)
                                            && this.contents_thumbnails.state(&id) == Some(super::contents_thumbnails::ThumbnailState::Running) => {
                                            // The re-fetch answered the same content
                                            // key the shown image already holds — the
                                            // PNG is neither read nor decoded; the
                                            // entry keeps its image and re-enters
                                            // Ready.
                                            if this.thumbnail_images.get(&id).is_some_and(|(kept, _)| kept == &key) {
                                                this.contents_thumbnails.ready_kept(&id);
                                                this.thumbnail_slot_active = false;
                                                this.acknowledge_thumbnail(request_id, cx);
                                                cx.notify();
                                            } else {
                                            let entity = cx.entity();
                                            let decoder = cx.background_executor().spawn(async move { decode_preview_png(&path, super::contents_thumbnails::THUMBNAIL_SIZE) });
                                            cx.spawn(async move |_, cx| {
                                                let decoded = decoder.await;
                                                entity.update(cx, |session, cx| {
                                                    let current_project = session.project_value().to_string();
                                                    if let Ok(rgba) = decoded
                                                        && id.project == current_project
                                                        && id.generation == session.contents_thumbnails.generation()
                                                        && session.contents_lists(&id)
                                                    {
                                                        let bytes = rgba.as_raw().len();
                                                        let image = render_image(rgba);
                                                        if let Some(evicted) = session.contents_thumbnails.ready(&id, bytes) {
                                                            for old in evicted {
                                                                if let Some((_, old_image)) = session.thumbnail_images.remove(&old) {
                                                                    session.pending_thumbnail_drops.push(old_image);
                                                                }
                                                            }
                                                            if session.contents_thumbnails.state(&id)
                                                                == Some(super::contents_thumbnails::ThumbnailState::Ready)
                                                            {
                                                                // The replaced image (a kept
                                                                // thumbnail whose key changed)
                                                                // goes to the release list, not
                                                                // an in-place drop.
                                                                if let Some((_, prior)) = session.thumbnail_images.remove(&id) {
                                                                    session.pending_thumbnail_drops.push(prior);
                                                                }
                                                                session.thumbnail_images.insert(id.clone(), (key, image));
                                                            } else {
                                                                session.pending_thumbnail_drops.push(image);
                                                            }
                                                        }
                                                    } else { session.contents_thumbnails.failed(&id); }
                                                    session.thumbnail_slot_active = false;
                                                    session.acknowledge_thumbnail(request_id, cx);
                                                    cx.notify();
                                                });
                                            }).detach();
                                            }
                                        }
                                        // A reply the list no longer wants (another project,
                                        // or its row is gone) must not leave the thumbnail
                                        // Running forever: fail it for this revision.
                                        Ok(_) => {
                                            if this.contents_thumbnails.state(&id)
                                                == Some(super::contents_thumbnails::ThumbnailState::Running)
                                            {
                                                this.contents_thumbnails.failed(&id);
                                            }
                                            this.thumbnail_slot_active = false;
                                            this.acknowledge_thumbnail(request_id, cx);
                                        }
                                    }
                                }
                                Update::ParameterValues {
                                    project,
                                    form,
                                    group_page,
                                    page,
                                } => {
                                    // Only the current project's answer
                                    // lands; anything else stays a no-show.
                                    if let Some(entry) = this.values_pages.get_mut(&form)
                                        && entry.current(&project)
                                    {
                                        entry.answer(group_page, page);
                                        cx.notify();
                                    }
                                }
                                Update::SpectrumTable {
                                    project,
                                    resource_id,
                                    table,
                                } => {
                                    if let Some(entry) =
                                        this.spectrum_answers.get_mut(&resource_id)
                                        && entry.current(&project)
                                    {
                                        entry.table = Asked::Done(table);
                                        cx.notify();
                                    }
                                }
                                Update::SpectrumSwatch {
                                    project,
                                    resource_id,
                                    role,
                                    answer,
                                } => {
                                    if let Some(entry) =
                                        this.spectrum_answers.get_mut(&resource_id)
                                        && entry.current(&project)
                                    {
                                        entry.swatches.insert(role, Asked::Done(answer));
                                        cx.notify();
                                    }
                                }
                                Update::ParameterSpectrumColour {
                                    project,
                                    target,
                                    parameter,
                                    role,
                                    answer,
                                } => {
                                    // Only the current project's answer
                                    // lands, in the slot the role keyed.
                                    if let Some(id) = target["id"].as_str()
                                        && let Some(entry) = this.parameter_answers.get_mut(id)
                                        && entry.current(&project)
                                    {
                                        entry.swatches.insert(
                                            swatch_slot(&parameter, role.as_deref()),
                                            Asked::Done(answer),
                                        );
                                        cx.notify();
                                    }
                                }
                                Update::ResourceDetails {
                                    project,
                                    resource_id,
                                    details,
                                } => {
                                    this.resource_details
                                        .insert(resource_id, (project, details));
                                }
                            }
                        }
                        let now = std::time::Instant::now();
                        changed |= this.render_start.expire(now);
                        let active_ids = this.foreground_jobs["jobs"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|job| job["job_id"].as_str().map(str::to_owned))
                            .collect::<std::collections::HashSet<_>>();
                        for id in &active_ids {
                            this.foreground_seen.entry(id.clone()).or_insert(now);
                        }
                        this.foreground_seen.retain(|id, _| active_ids.contains(id));
                        let visible = this.foreground_seen.values().any(|start| {
                            super::jobs::foreground_debounce_elapsed(*start, now)
                        });
                        if visible != this.foreground_visible {
                            this.foreground_visible = visible;
                            changed = true;
                        }
                        if let Some(backend::FrameUpdate {
                            id,
                            frame,
                            camera,
                            ..
                        }) = this.backend.take_frame(&this.scene_key)
                        {
                            changed = true;
                            let started = std::time::Instant::now();
                            this.image_size = [frame.width, frame.height];
                            this.view_camera = Some(camera);
                            // The GPU worker delivers rgba (outline and the
                            // channel conversion both moved there), so this
                            // measures only the RenderImage construction and
                            // the image swap below.
                            if let Some(buffer) =
                                image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
                            {
                                let image =
                                    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
                                if let Some(old) = this.image.replace(image) {
                                    let _ = window.drop_image(old);
                                }
                                this.frame_id = id;
                                this.updating = false;
                                this.stale_frame = false;
                                this.preview_error = None;
                                backend::preview_timing::record(
                                    "gpui_image",
                                    started,
                                    None,
                                    &this.scene_key,
                                    Some(id),
                                );
                            }
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let mut this = Self {
            layout_revision: 0,
            jobs: serde_json::json!({}),
            foreground_jobs: serde_json::json!({"jobs": []}),
            foreground_seen: std::collections::HashMap::new(),
            foreground_visible: false,
            settings: serde_json::json!({}),
            ipr: serde_json::json!({}),
            render_image: None,
            render_artifact: None,
            render_camera_label: None,
            render_project: serde_json::Value::Null,
            render_mean_luminance: None,
            render_received: None,
            render_start: Default::default(),
            backend,
            scene_key: "empty".into(),
            messages: Vec::new(),
            status: "Open a project or import a PBRT scene to begin".into(),
            image: None,
            frame_id: 0,
            image_size: [1280, 720],
            view_camera: None,
            updating: false,
            stale_frame: false,
            preview_error: None,
            gizmo_reset: false,
            preview_mode: PreviewMode::Solid,
            time_slider,
            supported_times: vec![0.],
            view_time: 0.,
            project: super::document::empty_project_state(),
            project_tree: serde_json::json!({"nodes":[]}),
            tree_scope: String::new(),
            project_search_query: String::new(),
            project_search: serde_json::Value::Null,
            contents_search_query: String::new(),
            contents_search: serde_json::Value::Null,
            contents_search_project: serde_json::Value::Null,
            contents_search_generation: 0,
            contents_search_error: None,
            contents_search_in_flight: None,
            contents_search_settled: None,
            contents: Default::default(),
            contents_thumbnails: Default::default(),
            thumbnail_images: Default::default(),
            pending_thumbnail_drops: Vec::new(),
            thumbnail_slot_active: false,
            thumbnail_done_pending: None,
            thumbnail_visible_at: None,
            project_scenes: serde_json::Value::Null,
            project_presets: serde_json::Value::Null,
            resource_schema: serde_json::Value::Null,
            resource_graphs: Default::default(),
            resource_details: Default::default(),
            node_positions: Default::default(),
            node_positions_file: None,
            node_positions_loaded: 0,
            previews: Default::default(),
            values_page: Default::default(),
            values_pages: Default::default(),
            spectrum_answers: Default::default(),
            parameter_answers: Default::default(),
            item_references: Default::default(),
            reveal_where_used: None,
            children_arrived: Vec::new(),
            reveal_created: None,
            reveal_picked: None,
            selected_project_node: None,
            project_inspector: serde_json::json!({"message":"Select a project node"}),
            graph_value: None,
            node_editor_creates: Default::default(),
            node_editor_create_key: 0,
            inspect_asked: None,
            save_settled: None,
            save_in_flight: false,
            unsaved_changes: false,
            import_warnings: 0,
            folded: Default::default(),
            folded_path: None,
            transform_matrix: false,
            edit_feedback: Default::default(),
            colour_dialogs: Default::default(),
            editors_ready: Vec::new(),
            items_missing: Vec::new(),
        };
        // r4's parameter table is static: fetch the
        // Contents ▸ New rows once at startup.
        this.send(Command::GetResourceSchemas, cx);
        this
    }

    /// Fold or unfold one component kind everywhere and remember it.
    pub(super) fn toggle_folded(&mut self, kind: &str) {
        self.folded.toggle(kind);
        if let Some(path) = &self.folded_path {
            let _ = self.folded.save(path);
        }
    }

    pub(super) fn unfold_component(&mut self, kind: &str) {
        self.folded.unfold(kind);
        if let Some(path) = &self.folded_path {
            let _ = self.folded.save(path);
        }
    }
}

#[cfg(test)]
mod thumbnail_decode_tests {
    use super::{decode_preview_png, mean_luminance, render_image};

    #[test]
    fn thumbnail_decode_requires_bounded_png_bytes_and_exact_dimensions() {
        let directory = tempfile::tempdir().unwrap();
        let valid = directory.path().join("valid.png");
        image::RgbaImage::new(64, 64).save(&valid).unwrap();
        assert_eq!(
            decode_preview_png(
                valid.to_str().unwrap(),
                super::super::contents_thumbnails::THUMBNAIL_SIZE
            )
            .unwrap()
            .dimensions(),
            (64, 64)
        );

        let wrong_size = directory.path().join("wrong.png");
        image::RgbaImage::new(65, 64).save(&wrong_size).unwrap();
        assert!(
            decode_preview_png(
                wrong_size.to_str().unwrap(),
                super::super::contents_thumbnails::THUMBNAIL_SIZE
            )
            .is_err()
        );

        let too_large = directory.path().join("large.png");
        std::fs::write(&too_large, vec![0; 2 * 1024 * 1024 + 1]).unwrap();
        assert!(
            decode_preview_png(
                too_large.to_str().unwrap(),
                super::super::contents_thumbnails::THUMBNAIL_SIZE
            )
            .is_err()
        );
    }

    /// The Inspector preview path decodes 128px PNGs and refuses
    /// anything smaller, so a Contents-size 64px file can never be pinned as
    /// an Inspector preview.
    #[test]
    fn inspector_preview_decode_accepts_128px_and_rejects_smaller() {
        let directory = tempfile::tempdir().unwrap();
        let inspector = directory.path().join("inspector.png");
        image::RgbaImage::new(128, 128).save(&inspector).unwrap();
        assert_eq!(
            decode_preview_png(inspector.to_str().unwrap(), 128)
                .unwrap()
                .dimensions(),
            (128, 128)
        );
        let contents_size = directory.path().join("contents-size.png");
        image::RgbaImage::new(64, 64).save(&contents_size).unwrap();
        assert!(decode_preview_png(contents_size.to_str().unwrap(), 128).is_err());
    }

    #[test]
    fn inspector_preview_decode_still_serves_contents_at_64px() {
        let directory = tempfile::tempdir().unwrap();
        let valid = directory.path().join("valid.png");
        image::RgbaImage::new(64, 64).save(&valid).unwrap();
        assert_eq!(
            decode_preview_png(
                valid.to_str().unwrap(),
                super::super::contents_thumbnails::THUMBNAIL_SIZE
            )
            .unwrap()
            .dimensions(),
            (64, 64)
        );
        assert_eq!(super::super::contents_thumbnails::THUMBNAIL_SIZE, 64);
        assert_eq!(super::INSPECTOR_PREVIEW_SIZE, 128);
    }

    #[test]
    fn mean_luminance_uses_rgb_channels_and_ignores_alpha() {
        assert!((mean_luminance(&[255, 255, 255, 0]).unwrap() - 1.).abs() < 1e-12);
        assert_eq!(mean_luminance(&[0, 0, 0, 255]), Some(0.));
        assert!(mean_luminance(&[]).is_none());
    }

    #[test]
    fn render_image_bgra_swaps_pixels() {
        let mut rgba = image::RgbaImage::new(1, 1);
        rgba.put_pixel(0, 0, image::Rgba([1, 2, 3, 4]));
        let image = render_image(rgba);
        assert_eq!(image.as_bytes(0).unwrap(), &[3, 2, 1, 4]);
    }
}

#[cfg(test)]
mod preview_lifetime_tests {
    use super::{same_preview_owner, scene_geometry_interactive};
    use protocol::{SceneId, project::ProjectRef};
    use serde_json::json;

    fn project(revision: &str, activation: &str) -> ProjectRef {
        serde_json::from_value(json!({
            "project_id":"00000000-0000-0000-0000-000000000001",
            "revision":revision,
            "activation":activation
        }))
        .unwrap()
    }

    fn scene(value: &str) -> SceneId {
        value.parse().unwrap()
    }

    #[test]
    fn image_owner_survives_revision_updates_but_not_scene_or_activation_switches() {
        let scene_a = scene("00000000-0000-0000-0000-000000000002");
        let scene_b = scene("00000000-0000-0000-0000-000000000003");
        let old = project("1", "1");
        assert!(same_preview_owner(
            Some(&old),
            Some(scene_a),
            Some(&project("2", "1")),
            Some(scene_a)
        ));
        assert!(!same_preview_owner(
            Some(&old),
            Some(scene_a),
            Some(&project("2", "1")),
            Some(scene_b)
        ));
        assert!(!same_preview_owner(
            Some(&old),
            Some(scene_a),
            Some(&project("1", "2")),
            Some(scene_a)
        ));
    }

    #[test]
    fn stale_and_updating_frames_are_never_geometry_interactive() {
        assert!(scene_geometry_interactive(false, false));
        assert!(!scene_geometry_interactive(true, false));
        assert!(!scene_geometry_interactive(false, true));
    }
}

/// The Inspector/node-editor preview state machine, plain
/// functions so the request/backoff/land/evict logic runs without GPUI.
#[cfg(test)]
mod preview_state_tests {
    use super::{MAX_PREVIEW_IMAGES, Preview, evict_preview_images, render_image};
    use gpui_kit::RenderImage;
    use serde_json::json;
    use std::{
        collections::HashMap,
        sync::Arc,
        time::{Duration, Instant},
    };

    fn one_image() -> Arc<RenderImage> {
        render_image(image::RgbaImage::new(1, 1))
    }

    #[test]
    fn preview_asks_once_per_revision_and_backs_off_after_busy() {
        let project = json!({"project_id":"p","revision":"r1","activation":"a"});
        let now = Instant::now();
        let mut entry = Preview::default();
        assert!(
            entry.wants_request(&project, now),
            "an unasked entry requests"
        );
        entry.mark_requested(&project);
        assert_eq!(entry.at, project);
        assert!(
            !entry.wants_request(&project, now),
            "the same revision is asked at most once"
        );
        assert!(
            entry.wants_request(
                &json!({"project_id":"p","revision":"r2","activation":"a"}),
                now
            ),
            "a new revision re-asks"
        );
        entry.retry_busy(now);
        assert!(entry.at.is_null(), "a Busy backoff re-opens the request");
        assert!(!entry.wants_request(&project, now), "still backing off");
        assert!(
            entry.wants_request(&project, now + Duration::from_millis(250)),
            "the 250ms backoff elapses"
        );
        entry.mark_requested(&project);
        assert_eq!(entry.at, project);
    }

    #[test]
    fn preview_keeps_its_image_while_rerequesting_and_releases_stale_ones() {
        let old_project = json!({"project_id":"p","revision":"r1","activation":"a"});
        let new_project = json!({"project_id":"p","revision":"r2","activation":"a"});
        let kept = one_image();
        let kept_pointer = Arc::clone(&kept);
        let mut entry = Preview {
            at: old_project.clone(),
            retry_at: None,
            image: Some(kept),
            used: None,
            key: None,
        };
        // A re-request at a new revision keeps the old picture on screen.
        entry.mark_requested(&new_project);
        assert!(entry.image.is_some(), "the old image stays while re-asking");
        // A decode for the abandoned revision is handed back for release.
        let stale = one_image();
        let stale_pointer = Arc::clone(&stale);
        let released = entry
            .land_image(&old_project, "k1".to_owned(), stale)
            .unwrap();
        assert!(Arc::ptr_eq(&released, &stale_pointer));
        assert!(
            entry
                .image
                .as_ref()
                .is_some_and(|image| Arc::ptr_eq(image, &kept_pointer)),
            "an abandoned decode never replaces the shown image"
        );
        // The fresh decode for the current revision replaces the old image;
        // the replaced one is handed back for release.
        let replaced = entry
            .land_image(&new_project, "k1".to_owned(), one_image())
            .unwrap();
        assert!(Arc::ptr_eq(&replaced, &kept_pointer));
        assert!(entry.image.is_some(), "the fresh image lands");
    }

    #[test]
    fn preview_superseded_by_a_new_revision_keeps_its_image_and_reasks() {
        let old_project = json!({"project_id":"p","revision":"r1","activation":"a"});
        let new_project = json!({"project_id":"p","revision":"r2","activation":"a"});
        let mut entry = Preview {
            at: old_project.clone(),
            retry_at: None,
            image: Some(one_image()),
            used: None,
            key: None,
        };
        entry.superseded();
        assert!(
            entry.image.is_some(),
            "a stale reply never blanks the image"
        );
        assert!(entry.wants_request(&new_project, Instant::now()));
        assert!(entry.wants_request(&old_project, Instant::now()));
    }

    #[test]
    fn preview_failure_clears_the_image_but_keeps_the_revision() {
        let project = json!({"project_id":"p","revision":"r1","activation":"a"});
        let mut entry = Preview {
            at: project.clone(),
            retry_at: None,
            image: Some(one_image()),
            used: None,
            key: None,
        };
        assert!(entry.clear_image().unwrap().as_bytes(0).is_some());
        assert!(entry.image.is_none(), "a failed reply is a no-show");
        assert_eq!(entry.at, project, "no second ask within the revision");
    }

    #[test]
    fn preview_decode_failure_for_an_abandoned_revision_keeps_the_newer_image() {
        let old_project = json!({"project_id":"p","revision":"r1","activation":"a"});
        let new_project = json!({"project_id":"p","revision":"r2","activation":"a"});
        let kept = one_image();
        let kept_pointer = Arc::clone(&kept);
        let mut entry = Preview {
            at: new_project.clone(),
            retry_at: None,
            image: Some(kept),
            used: None,
            key: None,
        };
        // The r1 decode fails after the entry moved on to: the
        // picture on screen must survive the abandoned failure.
        assert!(entry.reject_image(&old_project).is_none());
        assert!(
            entry
                .image
                .as_ref()
                .is_some_and(|image| Arc::ptr_eq(image, &kept_pointer)),
            "an abandoned decode failure never blanks the shown image"
        );
        assert_eq!(entry.at, new_project);
        // A failure for the revision the entry is showing is a no-show.
        let released = entry.reject_image(&new_project).unwrap();
        assert!(Arc::ptr_eq(&released, &kept_pointer));
        assert!(entry.image.is_none());
        assert_eq!(entry.at, new_project);
    }

    #[test]
    fn preview_lru_evicts_the_least_recently_used_idle_image_and_reopens_asking() {
        let project = json!({"project_id":"p","revision":"r1","activation":"a"});
        let now = Instant::now();
        // Every entry was last drawn long ago, in order.
        let idle = |index: usize| {
            Some(now - Duration::from_secs(60) + Duration::from_millis(index as u64))
        };
        let mut previews = HashMap::new();
        let mut drops = Vec::new();
        for index in 0..=MAX_PREVIEW_IMAGES {
            let entry = Preview {
                at: project.clone(),
                retry_at: None,
                image: Some(one_image()),
                used: idle(index),
                key: None,
            };
            previews.insert(format!("p{index}"), entry);
        }
        evict_preview_images(&mut previews, &mut drops, now);
        assert_eq!(drops.len(), 1, "only the over-budget image is released");
        assert!(
            drops.first().unwrap().as_bytes(0).is_some(),
            "the evicted image goes to the release list"
        );
        let victim = previews.get("p0").unwrap();
        assert!(
            victim.image.is_none(),
            "the least recently used entry loses its image"
        );
        assert!(
            victim.at.is_null(),
            "the evicted entry re-asks when it is shown again"
        );
        assert_eq!(
            previews
                .values()
                .filter(|entry| entry.image.is_some())
                .count(),
            MAX_PREVIEW_IMAGES
        );
        // A newcomer drawn just now evicts the next least used idle keeper,
        // never itself.
        let newcomer = Preview {
            at: project.clone(),
            retry_at: None,
            image: Some(one_image()),
            used: Some(now),
            key: None,
        };
        previews.insert("fresh".into(), newcomer);
        evict_preview_images(&mut previews, &mut drops, now);
        assert_eq!(drops.len(), 2);
        assert!(
            previews["p1"].image.is_none(),
            "the next least used is evicted"
        );
        assert!(previews["fresh"].image.is_some());
    }

    /// The reply's content key decides whether the PNG is worth
    /// reading again — an unchanged key means the shown image already is
    /// this revision's answer.
    #[test]
    fn preview_reply_with_unchanged_content_key_keeps_the_image_without_decoding() {
        let old_project = json!({"project_id":"p","revision":"r1","activation":"a"});
        let new_project = json!({"project_id":"p","revision":"r2","activation":"a"});
        let kept = one_image();
        let mut entry = Preview {
            at: old_project.clone(),
            retry_at: None,
            image: None,
            used: None,
            key: None,
        };
        assert!(
            !entry.keeps_image(&old_project, "k1"),
            "nothing is kept before an image lands"
        );
        entry.land_image(&old_project, "k1".to_owned(), kept);
        assert!(
            entry.keeps_image(&old_project, "k1"),
            "a reply with the same content key keeps the shown image"
        );
        assert!(
            !entry.keeps_image(&old_project, "k2"),
            "a changed content key reads and decodes again"
        );
        // The revision moves on and re-asks; the reply's key decides. The
        // held key matches the upcoming answer, so the PNG is not re-read.
        entry.mark_requested(&new_project);
        assert!(
            entry.keeps_image(&new_project, "k1"),
            "the held key answers the new revision's reply without a re-read"
        );
        // A changed image replaces the held one and answers the new key.
        entry.land_image(&new_project, "k2".to_owned(), one_image());
        assert!(!entry.keeps_image(&new_project, "k1"));
        assert!(entry.keeps_image(&new_project, "k2"));
        // An evicted image is not a kept answer, whatever the key says.
        entry.image = None;
        assert!(!entry.keeps_image(&new_project, "k2"));
    }

    /// The decoded thumbnails survive a same-activation revision
    /// under their new keys, so the panel keeps showing them while the
    /// re-fetch confirms their content keys.
    #[test]
    fn revise_thumbnail_images_rekeys_to_the_new_revision() {
        use super::super::contents_thumbnails::ThumbnailId;
        let (old_project, new_project) = (
            json!({"project_id":"p","activation":"a","revision":"r1"}),
            json!({"project_id":"p","activation":"a","revision":"r2"}),
        );
        let old_id = ThumbnailId {
            project: old_project.to_string(),
            id: "a".into(),
            kind: crate::backend::PreviewKind::Texture,
            generation: 0,
        };
        let mut images = std::collections::HashMap::new();
        images.insert(old_id, ("k1".to_owned(), one_image()));
        super::revise_thumbnail_images(&mut images, &new_project.to_string(), 1);
        let rekeyed = ThumbnailId {
            project: new_project.to_string(),
            id: "a".into(),
            kind: crate::backend::PreviewKind::Texture,
            generation: 1,
        };
        assert!(
            images.get(&rekeyed).is_some_and(|(key, _)| key == "k1"),
            "the held image and its content key survive under the new key"
        );
    }

    /// PR #297 review: a node editor draws every graph node (up to 256).
    /// Evicting an on-screen preview made it re-ask on the next render and
    /// evict another, forever; on-screen previews now stay over the cap.
    #[test]
    fn previews_on_screen_are_never_evicted_even_over_the_cap() {
        let project = json!({"project_id":"p","revision":"r1","activation":"a"});
        let now = Instant::now();
        let mut previews = HashMap::new();
        let mut drops = Vec::new();
        for index in 0..MAX_PREVIEW_IMAGES * 2 {
            let entry = Preview {
                at: project.clone(),
                retry_at: None,
                image: Some(one_image()),
                used: Some(now - Duration::from_millis(index as u64)),
                key: None,
            };
            previews.insert(format!("p{index}"), entry);
        }
        evict_preview_images(&mut previews, &mut drops, now);
        assert!(drops.is_empty(), "nothing on screen is released");
        assert!(previews.values().all(|entry| entry.at == project));
        // Once the editor closes and they go idle, the cap applies again.
        let later = now + Duration::from_secs(5);
        evict_preview_images(&mut previews, &mut drops, later);
        assert_eq!(drops.len(), MAX_PREVIEW_IMAGES);
        assert_eq!(
            previews
                .values()
                .filter(|entry| entry.image.is_some())
                .count(),
            MAX_PREVIEW_IMAGES
        );
    }
}

/// The parameter colour answers' single-flight state — one request
/// per visible row per revision, cached until the revision moves.
#[cfg(test)]
mod parameter_colour_cache_tests {
    use super::{Asked, SpectrumAnswers};
    use serde_json::json;

    #[test]
    fn parameter_colours_are_queued_once_per_project_and_cached() {
        let mut answers = SpectrumAnswers::default();
        let first = json!({"project_id":1,"revision":"a"});
        // The first render of a visible row queues exactly one command…
        assert!(answers.asking(&first, "L"));
        answers.mark_pending("L");
        assert!(
            !answers.asking(&first, "L"),
            "while the answer is pending, no second command may flood the core"
        );
        // …and no colour shows until the answer lands.
        assert_eq!(answers.swatch_answer("L"), None);
        answers.swatches.insert(
            "L".into(),
            Asked::Done(Ok(json!({"rgb":[1.,0.,0.],"source":"blackbody 6500 K"}))),
        );
        assert_eq!(
            answers.swatch_answer("L"),
            Some(json!({"rgb":[1.,0.,0.],"source":"blackbody 6500 K"}))
        );
        assert!(
            !answers.asking(&first, "L"),
            "a landed answer never re-asks"
        );
        // Each visible row asks independently within the same revision.
        assert!(answers.asking(&first, "reflectance"));
        // A new revision resets the row: one fresh command, and the old
        // answer is dropped rather than shown for the wrong revision.
        let second = json!({"project_id":1,"revision":"b"});
        assert!(answers.asking(&second, "L"));
        assert_eq!(answers.swatch_answer("L"), None);
    }

    #[test]
    fn a_command_the_queue_refused_asks_again_on_the_next_render() {
        let mut answers = SpectrumAnswers::default();
        let project = json!({"project_id":1,"revision":"a"});
        // The queue was full: the slot was never marked pending, so the
        // next render queues the command again instead of waiting forever.
        assert!(answers.asking(&project, "L"));
        assert!(answers.asking(&project, "L"));
        answers.mark_pending("L");
        assert!(!answers.asking(&project, "L"));
    }

    #[test]
    fn a_refused_answer_surfaces_as_error_json() {
        let mut answers = SpectrumAnswers::default();
        answers
            .swatches
            .insert("L".into(), Asked::Done(Err("parameter not found".into())));
        assert_eq!(
            answers.swatch_answer("L"),
            Some(json!({"error":"parameter not found"}))
        );
    }

    /// The cache slot keys the parameter *with* its role, so one
    /// resource keeps its albedo box and its Metal colour apart — the
    /// same parameter, separate slots and commands.
    #[test]
    fn slots_key_the_parameter_with_the_role() {
        use super::swatch_slot;
        assert_eq!(swatch_slot("eta", None), "eta");
        assert_eq!(swatch_slot("eta", Some("chromaticity")), "eta#chromaticity");
        assert_eq!(
            swatch_slot("conductor.k", Some("conductor")),
            "conductor.k#conductor"
        );
        let mut answers = SpectrumAnswers::default();
        let project = json!({"project_id":1,"revision":"a"});
        // Both eta reads of one row stay in flight at once.
        assert!(answers.asking(&project, "eta"));
        answers.mark_pending("eta");
        assert!(answers.asking(&project, "eta#chromaticity"));
        assert!(!answers.asking(&project, "eta"));
        // And the answers land in their own slots without clobbering.
        answers.swatches.insert(
            "eta#chromaticity".into(),
            Asked::Done(Ok(json!({"rgb":[1.,0.5,0.2]}))),
        );
        answers
            .swatches
            .insert("eta".into(), Asked::Done(Err("not found".into())));
        assert_eq!(
            answers.swatch_answer("eta#chromaticity"),
            Some(json!({"rgb":[1.,0.5,0.2]}))
        );
        assert_eq!(
            answers.swatch_answer("eta"),
            Some(json!({"error":"not found"}))
        );
    }
}

#[cfg(test)]
mod deselect_tests {
    use super::deselect_state;
    use serde_json::json;

    #[test]
    fn deselect_retires_a_selected_node_and_keeps_other_pages() {
        let details = json!({"node": {"node_id": "shape-7"}});
        // A live scene node retires: no marking anywhere, and the
        // Inspector's page becomes the message a pick that hits
        // nothing shows.
        let (node, page) = deselect_state(Some("shape-7".into()), details);
        assert_eq!(node, None);
        assert_eq!(page["message"], "Select a project node");
        // No node holds the selection — a Contents item or a value
        // page keeps the page it has.
        let item = json!({"item": {"id": "gold"}});
        assert_eq!(deselect_state(None, item.clone()), (None, item));
    }
}
