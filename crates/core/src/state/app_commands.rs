//! Command handling within the single CoreState owner.
use super::*;
impl CoreState {
    pub(super) fn handle_app(&mut self, request: &Request) -> Result<Value, ErrorPayload> {
        match request.method {
            Method::AppHello => {
                let p: HelloParams = params(request)?;
                if p.client_name.is_empty() {
                    return Err(invalid("client_name must not be empty"));
                }
                if !p
                    .supported_protocols
                    .iter()
                    .any(|version| version == protocol::PROTOCOL_VERSION)
                {
                    return Err(ErrorPayload::new(
                        ErrorCode::UnsupportedProtocol,
                        "no supported protocol in common",
                    ));
                }
                Ok(
                    json!({"session_id":self.session_id,"protocol":protocol::PROTOCOL_VERSION,
                    "capabilities":self.capabilities,"limits":self.limits}),
                )
            }
            Method::AppGetState => {
                let _: EmptyParams = params(request)?;
                Ok(json!(self.snapshot()))
            }
            Method::AppGetCapabilities => {
                let _: EmptyParams = params(request)?;
                Ok(json!(self.capabilities))
            }
            Method::AppQuit => {
                let p: QuitParams = params(request)?;
                if let Some(error) = self
                    .projects
                    .unsaved_changes(p.discard_unsaved, self.capture_attached())
                {
                    return Err(error);
                }
                self.quit_requested = true;
                Ok(json!({"quitting":true}))
            }
            _ => unreachable!("command routed to the wrong handler"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::project_commands::IoOutput;

    fn request(method: Method, params: Value) -> Request {
        Request {
            protocol: protocol::PROTOCOL_VERSION.into(),
            request_id: Uuid::new_v4().to_string(),
            method,
            params,
        }
    }

    /// `app.quit` guards unsaved edits like `project.close`, and once
    /// accepted the state says so.
    #[test]
    fn quit_refuses_unsaved_edits_unless_discarded() {
        let mut core = CoreState::new(Limits::default());
        let state = core
            .handle(request(Method::AppGetState, json!({})))
            .result
            .unwrap();
        assert_eq!(state["quit_requested"], false);
        let context = state["project"]["context"].clone();
        let open = core.handle(request(
            Method::ProjectOpen,
            json!({"path":crate::TEST_PROJECT,"expected_current":context,"discard_unsaved":true}),
        ));
        let id = serde_json::from_value(open.result.unwrap()["job_id"].clone()).unwrap();
        let snapshot = project::ProjectSession::new(project::ProjectDocument::new("Native"))
            .unwrap()
            .snapshot();
        core.project_io_finished(
            id,
            Ok(IoOutput::Loaded {
                snapshot,
                store_id: Uuid::new_v4(),
                path: Some(crate::TEST_PROJECT.into()),
                source_path: None,
                unregistered: vec![],
                published: false,
                report: None,
                mode: protocol::project::ProjectMode::Project,
                asset_root: None,
                staging_owner: None,
            }),
        );
        assert_eq!(
            core.handle(request(Method::AppQuit, json!({})))
                .result
                .unwrap()["quitting"],
            true,
            "a clean project quits"
        );
        core.quit_requested = false;
        let applied = core.handle(request(
            Method::ProjectApply,
            json!({"project":core.projects.reference().unwrap(),
                "edits":[{"kind":"create_scene","location":"Second.scene"}]}),
        ));
        assert!(applied.error.is_none(), "{:?}", applied.error);
        let refused = core.handle(request(Method::AppQuit, json!({})));
        assert_eq!(refused.error.unwrap().code, ErrorCode::UnsavedChanges);
        assert_eq!(
            core.handle(request(Method::AppGetState, json!({})))
                .result
                .unwrap()["quit_requested"],
            false
        );
        let accepted = core.handle(request(Method::AppQuit, json!({"discard_unsaved":true})));
        assert_eq!(accepted.result.unwrap()["quitting"], true);
        assert_eq!(
            core.handle(request(Method::AppGetState, json!({})))
                .result
                .unwrap()["quit_requested"],
            true
        );
    }
}
