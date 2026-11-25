//! Command handling within the single CoreState owner.
use super::*;
impl CoreState {
    pub(super) fn handle_workspace(&mut self, request: &Request) -> Result<Value, ErrorPayload> {
        match request.method {
            Method::WorkspaceResetLayout => {
                let _: EmptyParams = params(request)?;
                if !self.capabilities.workspace_layout {
                    return Err(ErrorPayload::new(
                        ErrorCode::Unsupported,
                        "headless host has no layout",
                    ));
                }
                self.layout_revision += 1;
                let result = json!({"layout_revision":self.layout_revision.to_string()});
                self.emit("workspace.layout_reset", None, None, result.clone());
                Ok(result)
            }
            _ => unreachable!("command routed to the wrong handler"),
        }
    }
}
