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
        let Some(due) = self.view_due else {
            self.watch_shared_view();
            return;
        };
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
        if self.adopt_external_camera(&current, &camera) {
            return;
        }
        let shared = &current["view"]["camera"];
        if current["view"].is_null() || *shared != camera {
            let written = request(
                &self.core,
                Method::ProjectSetCamera,
                json!({"project":project,"expected_view_revision":current["view_revision"],"camera":camera}),
            );
            if written.is_ok() {
                self.synced_camera = Some(camera);
            }
            // On a stale revision another client raced us; the next tick
            // refetches and adopts its camera.
            self.view_due = Some(Instant::now());
            return;
        }
        self.synced_camera = Some(camera);
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
    /// Read-only look at the shared view while nothing here moved: the app
    /// state carries no view revision, so poll for a camera another client
    /// set. It never writes, so a size, mode or time set by that client stays.
    fn watch_shared_view(&mut self) {
        let now = Instant::now();
        match self.view_watch_due {
            Some(due) if now < due => return,
            None => {
                self.view_watch_due = Some(now + Duration::from_millis(500));
                return;
            }
            Some(_) => self.view_watch_due = Some(now + Duration::from_millis(500)),
        }
        let project = self.last_project["context"]["project"].clone();
        if !project.is_object() || self.project_geometry.is_none() {
            return;
        }
        let (Ok(current), Ok(camera)) = (
            request(
                &self.core,
                Method::ProjectGetView,
                json!({"project":project}),
            ),
            serde_json::to_value(self.camera.to_protocol()),
        ) else {
            return;
        };
        self.adopt_external_camera(&current, &camera);
    }

    /// Take the shared camera when another client set it: it differs from the
    /// camera this display last wrote or adopted. Cameras are compared, not
    /// revisions, because size, mode and time changes also bump the revision.
    fn adopt_external_camera(&mut self, current: &Value, camera: &Value) -> bool {
        let shared = &current["view"]["camera"];
        if !self
            .synced_camera
            .as_ref()
            .is_some_and(|synced| synced != shared)
            || shared == camera
        {
            return false;
        }
        let Ok(wire_camera) = serde_json::from_value::<protocol::view::Camera>(shared.clone())
        else {
            return false;
        };
        let mut adopted = ViewerCamera::from_protocol(wire_camera);
        adopted.mirror = self.scene_mirror();
        self.camera = adopted;
        self.synced_camera = Some(shared.clone());
        self.dirty = true;
        // Re-render IPR from the new camera, but do not schedule a view write:
        // that would push this window's size, mode and time over the caller's.
        self.ipr_due = Some(Instant::now() + Duration::from_millis(300));
        true
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
