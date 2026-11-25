//! Command handling within the single CoreState owner.
use super::*;
impl CoreState {
    pub(super) fn handle_artifact(&mut self, request: &Request) -> Result<Value, ErrorPayload> {
        match request.method {
            Method::ArtifactGet => {
                let p: ArtifactParams = params(request)?;
                Ok(json!(self.artifacts.get(&p.artifact_id).ok_or_else(
                    || ErrorPayload::new(ErrorCode::NotFound, "artifact not found")
                )?))
            }
            Method::ArtifactExport => {
                let p: ExportParams = params(request)?;
                if !Path::new(&p.absolute_path).is_absolute() || p.absolute_path.contains('\0') {
                    return Err(invalid("export requires an absolute UTF-8 path"));
                }
                let artifact = self
                    .artifacts
                    .get(&p.artifact_id)
                    .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "artifact not found"))?
                    .clone();
                let (id, task) = self.service_task(
                    Work::Export {
                        artifact,
                        path: p.absolute_path,
                        overwrite: p.overwrite,
                    },
                    "export",
                )?;
                self.service_tasks.push_back(task);
                Ok(json!({"job_id":id}))
            }
            _ => unreachable!("command routed to the wrong handler"),
        }
    }
}
