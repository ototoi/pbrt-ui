//! Command handling within the single CoreState owner.
use super::*;
impl CoreState {
    pub(super) fn handle_events(&mut self, request: &Request) -> Result<Value, ErrorPayload> {
        match request.method {
            Method::EventsRead => {
                let p: ReadEventsParams = params(request)?;
                self.check_limit(p.limit)?;
                if p.wait_ms > self.limits.max_event_wait_ms {
                    return Err(invalid("wait_ms exceeds limit"));
                }
                let after = p
                    .after
                    .to_u64()
                    .ok_or_else(|| invalid("invalid event sequence"))?;
                let oldest = self
                    .events
                    .front()
                    .map_or(self.sequence + 1, |e| e.sequence.parse::<u64>().unwrap());
                if after < oldest.saturating_sub(1) {
                    return Err(ErrorPayload::new(
                        ErrorCode::ResyncRequired,
                        "event cursor expired",
                    )
                    .with_details(
                        json!({"oldest":oldest.to_string(),"latest":self.sequence.to_string()}),
                    ));
                }
                if after > self.sequence {
                    return Err(ErrorPayload::new(
                        ErrorCode::InvalidCursor,
                        "event cursor is in the future",
                    ));
                }
                let events: Vec<_> = self
                    .events
                    .iter()
                    .filter(|e| e.sequence.parse::<u64>().unwrap() > after)
                    .take(p.limit)
                    .collect();
                let next = events
                    .last()
                    .map_or(after.to_string(), |e| e.sequence.clone());
                Ok(json!({"events":events,"next_cursor":next}))
            }
            _ => unreachable!("command routed to the wrong handler"),
        }
    }
}
