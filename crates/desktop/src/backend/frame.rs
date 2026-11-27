//! Display adapter phase; state stays owned by Display.
use super::*;
impl Display {
    /// Recompute the outlined pick ids when the selection or the preview
    /// changed, redrawing for a new outline.
    fn sync_highlight(&mut self) {
        let key = (
            self.selected_project_node.clone(),
            self.project_preview_key.clone(),
            self.project_time,
        );
        if self.highlight_key.as_ref() == Some(&key) {
            return;
        }
        self.highlight_key = Some(key);
        let mut nodes = BTreeSet::new();
        if let Some(id) = self.selected_project_node.clone() {
            // A group outlines everything under it.
            if let Ok(descendants) =
                project_descendants(&self.core, &self.last_project["context"]["project"], &id)
            {
                nodes.extend(descendants);
            }
            nodes.insert(id);
        }
        let bounds = self.project_geometry.as_ref().and_then(|geometry| {
            crate::project_preview::selected_bounds(
                geometry,
                &self.project_pick_nodes,
                &self.project_object_bounds,
                self.project_time,
                &nodes,
            )
        });
        let _ = self.tx.send(Update::SelectionBounds(
            bounds.map(|bounds| bounds.center()),
        ));
        let highlight = self
            .project_pick_nodes
            .iter()
            .filter(|(_, node)| nodes.contains(*node))
            .map(|(pick, _)| *pick)
            .collect();
        if highlight != self.highlight {
            self.highlight = highlight;
            self.dirty = true;
        }
    }
    /// Take the pick ids of a newly uploaded preview.
    pub(super) fn install_pick_nodes(&mut self, bindings: BTreeMap<u32, String>) {
        self.project_pick_nodes = bindings;
        // The ids follow draw order, so the selection's ids may have moved
        // under an unchanged selection and preview key.
        self.highlight_key = None;
    }

    /// Receive the GPU worker's results. An install lands exactly where
    /// the old inline success branch put it; a render refreshes the pick
    /// state and publishes to the frames slot.
    ///
    /// Freshness is the request's echoed key against the tracked preview
    /// key — no per-frame Core call. UI `take_frame` keeps its
    /// strict scene-key match as the final defense.
    pub(super) fn poll_gpu(&mut self) {
        while let Ok(result) = self.gpu_rx.try_recv() {
            match result {
                gpu::GpuResult::Installed(data) => {
                    self.install_in_flight = false;
                    if data.key == self.project_preview_key.clone().unwrap_or_default() {
                        self.apply_installed(*data);
                    }
                    self.flush_queued_install();
                }
                gpu::GpuResult::InstallFailed {
                    key,
                    stage,
                    message,
                } => {
                    self.install_in_flight = false;
                    let prefix = match stage {
                        gpu::InstallStage::Parse => "Project preview parse",
                        gpu::InstallStage::Upload => "Project preview upload",
                    };
                    if key == self.project_preview_key.clone().unwrap_or_default() {
                        let text = format!("{prefix}: {message}");
                        self.fail_current_camera_preview();
                        let _ = self.tx.send(Update::PreviewUnavailable(Some(text.clone())));
                        let _ = self.tx.send(Update::Status(text));
                    }
                    self.flush_queued_install();
                }
                gpu::GpuResult::RenderedOk {
                    id,
                    key,
                    frame,
                    camera,
                } => {
                    self.render_in_flight = false;
                    if Some(&key) != self.project_preview_key.as_ref() {
                        continue;
                    }
                    self.frame_id = id;
                    // One pick buffer is retained; RGBA is sent once to GPUI.
                    self.last_frame = Some(PreviewFrame {
                        rgba: Vec::new(),
                        pick_ids: frame.pick_ids.clone(),
                        width: frame.width,
                        height: frame.height,
                        backend: frame.backend.clone(),
                    });
                    if let Ok(mut latest) = self.frames.lock() {
                        *latest = Some(FrameUpdate {
                            scene_key: key,
                            id,
                            frame,
                            camera,
                        });
                    }
                }
                gpu::GpuResult::RenderFailed { message } => {
                    self.render_in_flight = false;
                    self.last_frame = None;
                    let _ = self.tx.send(Update::Status(format!("Preview: {message}")));
                }
                gpu::GpuResult::LitTextures { key, notes } => {
                    // The render that ran the deferred lit texture
                    // stage reports its notes — evaluate's and the stage's
                    // own, the scene's current list. Log them through the
                    // install notes' dedup; a skipped key is a scene the
                    // display has already moved on from.
                    if Some(&key) == self.project_preview_key.as_ref() {
                        self.log_lit_notes(&notes);
                    }
                }
            }
        }
    }

    /// Turn a dirty display into a render request. The old serial
    /// `render_frame` ran here on this thread; the draw itself now belongs
    /// to the GPU worker, so the loop keeps its poll cadence while heavy
    /// preview work runs.
    pub(super) fn issue_gpu(&mut self) {
        self.sync_highlight();
        if !self.dirty || self.render_in_flight || !self.preview_ready {
            return;
        }
        let Some(key) = self.project_preview_key.clone() else {
            return;
        };
        let next = self.frame_id + 1;
        if self
            .gpu
            .requests
            .send(gpu::GpuRequest::Render {
                id: next,
                key,
                camera: self.camera,
                size: self.size,
                mode: self.mode,
                time: self.project_time,
                highlight: highlight_for_overlays(self.overlays_visible, &self.highlight),
            })
            .is_ok()
        {
            self.render_in_flight = true;
            self.dirty = false;
        }
    }
}

fn highlight_for_overlays(visible: bool, highlight: &BTreeSet<u32>) -> BTreeSet<u32> {
    if visible {
        highlight.clone()
    } else {
        BTreeSet::new()
    }
}

/// Tint the selected pixels (pick id in `selected`) and outline their
/// silhouette: selected pixels on its edge and unselected pixels touching
/// it are painted solid.
pub(super) fn outline_selection(
    rgba: &mut [u8],
    ids: &[u32],
    width: usize,
    selected: &BTreeSet<u32>,
) {
    const COLOR: [u8; 3] = [255, 160, 40];
    if selected.is_empty() || width == 0 || ids.len() * 4 != rgba.len() {
        return;
    }
    let height = ids.len() / width;
    let inside = |x: usize, y: usize| selected.contains(&ids[y * width + x]);
    for y in 0..height {
        for x in 0..width {
            let here = inside(x, y);
            let edge = [(0, -1), (0, 1), (-1, 0), (1, 0)].iter().any(|(dx, dy)| {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                    return false;
                }
                inside(nx as usize, ny as usize) != here
            });
            let pixel = &mut rgba[(y * width + x) * 4..][..3];
            if edge {
                pixel.copy_from_slice(&COLOR);
            } else if here {
                for (channel, color) in pixel.iter_mut().zip(COLOR) {
                    *channel = ((u16::from(*channel) * 3 + u16::from(color)) / 4) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod outline_tests {
    use super::*;

    #[test]
    fn hiding_overlays_suppresses_outline_without_changing_selection_ids() {
        let selected = BTreeSet::from([3, 7]);
        assert_eq!(highlight_for_overlays(true, &selected), selected);
        assert!(highlight_for_overlays(false, &selected).is_empty());
    }

    #[test]
    fn the_selection_is_tinted_and_outlined_on_both_sides_of_its_edge() {
        // 4x1: background, selected, selected, background.
        let ids = [0, 7, 7, 0];
        let mut rgba = vec![0u8; 16];
        outline_selection(&mut rgba, &ids, 4, &BTreeSet::from([7]));
        let pixels: Vec<_> = rgba.chunks(4).map(|p| [p[0], p[1], p[2]]).collect();
        assert_eq!(pixels, vec![[255, 160, 40]; 4]);

        // 5x1: the middle selected pixel is interior: tinted, not solid.
        let ids = [7, 7, 7, 0, 0];
        let mut rgba = vec![0u8; 20];
        outline_selection(&mut rgba, &ids, 5, &BTreeSet::from([7]));
        assert_eq!(&rgba[0..3], &[63, 40, 10]);
        assert_eq!(&rgba[4..7], &[63, 40, 10]);
        assert_eq!(&rgba[8..11], &[255, 160, 40]);
        assert_eq!(&rgba[12..15], &[255, 160, 40]);
        assert_eq!(&rgba[16..19], &[0, 0, 0]);

        // Nothing selected: untouched.
        let ids = [7, 7, 7, 0, 0];
        let mut rgba = vec![9u8; 20];
        outline_selection(&mut rgba, &ids, 5, &BTreeSet::new());
        assert!(rgba.iter().all(|v| *v == 9));
    }
}
