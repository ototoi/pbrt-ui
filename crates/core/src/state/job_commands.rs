//! Command handling within the single CoreState owner.
use super::*;
impl CoreState {
    pub(super) fn handle_jobs(&mut self, request: &Request) -> Result<Value, ErrorPayload> {
        match request.method {
            Method::JobGet => {
                let p: JobParams = params(request)?;
                Ok(json!(self.job(p.job_id)?))
            }
            Method::JobCancel => {
                let p: JobParams = params(request)?;
                self.job(p.job_id)?;
                self.ipr_job_cancelled(p.job_id);
                self.cancel_job(p.job_id);
                Ok(json!(self.job(p.job_id)?))
            }
            Method::JobList => {
                let p: ListJobsParams = params(request)?;
                self.check_limit(p.limit)?;
                let start = match p.cursor {
                    None => 0,
                    Some(cursor) => {
                        let (session, ordinal) = cursor.split_once(':').ok_or_else(|| {
                            ErrorPayload::new(ErrorCode::InvalidCursor, "invalid job cursor")
                        })?;
                        if session != self.session_id.to_string() {
                            return Err(ErrorPayload::new(
                                ErrorCode::InvalidCursor,
                                "cursor belongs to another session",
                            ));
                        }
                        ordinal
                            .parse::<usize>()
                            .ok()
                            .filter(|n| *n <= self.jobs.len())
                            .ok_or_else(|| {
                                ErrorPayload::new(ErrorCode::InvalidCursor, "invalid job ordinal")
                            })?
                    }
                };
                let end = start.saturating_add(p.limit).min(self.jobs.len());
                Ok(
                    json!({"jobs": self.jobs[start..end].iter().map(|r| &r.job).collect::<Vec<_>>(),
                    "next_cursor": (end < self.jobs.len()).then(|| format!("{}:{end}", self.session_id))}),
                )
            }
            _ => unreachable!("command routed to the wrong handler"),
        }
    }
}
