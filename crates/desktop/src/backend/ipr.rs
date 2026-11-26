//! Display adapter phase; state stays owned by Display.
use super::*;
impl Display {
    /// Size for renders, IPR and the shared view: the Scene viewport's
    /// aspect at a 1280-pixel long side, whatever the panel's pixels.
    pub(super) fn render_size(&self) -> [u32; 2] {
        let [width, height] = self.size.map(|side| f64::from(side.max(1)));
        let scale = 1280. / width.max(height);
        [
            ((width * scale).round() as u32).max(1),
            ((height * scale).round() as u32).max(1),
        ]
    }
    pub(super) fn arm_ipr_submit(&mut self) {
        let due = Instant::now() + Duration::from_millis(300);
        self.ipr_due = Some(due);
        // Camera edits also feed the project's shared native view after the
        // same debounce, so CLI/MCP captures frame the desktop's view.
        if self.last_project["context"]["project"].is_object() {
            self.view_due = Some(due);
        }
    }

    /// Push the live GUI view onto the shared native view when camera edits
    /// settle. The desktop's own picks, renders and IPR keeps pushing their
    /// exact explicit view; the shared copy serves external callers.
    pub(super) fn poll_view_sync(&mut self) {
        let Some(due) = self.view_due else { return };
        if Instant::now() < due {
            return;
        }
        self.view_due = None;
        let project = self.last_project["context"]["project"].clone();
        if !project.is_object() || self.project_geometry.is_none() {
            return;
        }
        let Ok(current) = request(
            &self.core,
            Method::ProjectGetView,
            json!({"project":project}),
        ) else {
            return;
        };
        let mode = match self.mode {
            PreviewMode::Solid => "solid",
            PreviewMode::Normal => "normal",
            PreviewMode::Wireframe => "wireframe",
            PreviewMode::Lit => "lit",
        };
        let camera = match serde_json::to_value(self.camera.to_protocol()) {
            Ok(camera) => camera,
            Err(_) => return,
        };
        if self.view_revision.as_ref() != Some(&current["view_revision"])
            && current["view"]["camera"] != camera
        {
            if let Ok(wire_camera) =
                serde_json::from_value::<protocol::view::Camera>(current["view"]["camera"].clone())
            {
                let mut adopted = ViewerCamera::from_protocol(wire_camera);
                adopted.mirror = self.scene_mirror();
                self.camera = adopted;
                self.view_revision = Some(current["view_revision"].clone());
                self.dirty = true;
                self.arm_ipr_submit();
                return;
            }
        }
        if current["view"].is_null() || current["view"]["camera"] != camera {
            let stale = request(
                &self.core,
                Method::ProjectSetCamera,
                json!({"project":project,"expected_view_revision":current["view_revision"],"camera":camera}),
            );
            if let Ok(result) = stale {
                self.view_revision = Some(result["view_revision"].clone());
                self.view_due = Some(Instant::now());
            } else {
                // Remote edits raced us; refetch the revision on the next tick.
                self.view_due = Some(Instant::now());
            }
            return;
        }
        if current["view"]["size"] != json!(self.render_size())
            || current["view"]["mode"] != json!(mode)
            || current["view"]["time"] != json!(self.project_time)
        {
            if request(
                &self.core,
                Method::ProjectConfigureView,
                json!({"project":project,"expected_view_revision":current["view_revision"],"size":self.render_size(),"mode":mode,"time":self.project_time}),
            )
            .is_err()
            {
                self.view_due = Some(Instant::now());
            }
        }
    }
    pub(super) fn poll_ipr_submit(&mut self) {
        let Some(due) = self.ipr_due else { return };
        if Instant::now() < due {
            return;
        }
        self.ipr_due = None;
        let project = &self.last_project["context"]["project"];
        let enabled = self.last_state["ipr"]["enabled"] == true;
        if !enabled || !project.is_object() || self.project_geometry.is_none() {
            return;
        }
        let view = json!({
            "camera":serde_json::to_value(self.camera.to_protocol()).unwrap_or(Value::Null),
            "size":self.render_size(),"mode":"solid","time":self.project_time,
        });
        if let Err(error) = request(
            &self.core,
            Method::RenderIprSubmit,
            json!({"project":project,"view":view}),
        ) {
            let _ = self
                .tx
                .send(Update::Status(format!("IPR preview: {error}")));
        }
    }
}
