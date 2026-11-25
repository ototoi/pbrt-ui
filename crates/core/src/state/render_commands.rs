//! Command handling within the single CoreState owner.
use super::*;
impl CoreState {
    pub(super) fn handle_render(&mut self, request: &Request) -> Result<Value, ErrorPayload> {
        match request.method {
            Method::RenderGetSettings => {
                let _: EmptyParams = params(request)?;
                Ok(
                    json!({"settings":self.settings,"settings_revision":self.settings_revision.to_string()}),
                )
            }
            Method::RenderSetSettings => {
                let p: SetRenderSettingsParams = params(request)?;
                revision(
                    &p.expected_settings_revision,
                    self.settings_revision,
                    "settings",
                )?;
                if p.settings.samples_per_pixel.is_some() {
                    return Err(invalid(
                        "Samples is configured by the scene Sampler; the render.settings samples_per_pixel override is deprecated",
                    ));
                }
                if p.settings.max_depth.is_some() {
                    return Err(invalid(
                        "Max depth is configured by the scene Integrator; the render.settings max_depth override is deprecated",
                    ));
                }
                if p.settings
                    .samples_per_pixel
                    .is_some_and(|n| n == 0 || n > 1_048_576)
                    || p.settings.threads.is_some_and(|n| n == 0 || n > 1024)
                    || p.settings.max_depth.is_some_and(|d| d == 0 || d > 1024)
                {
                    return Err(invalid("render settings exceed limits"));
                }
                self.settings = p.settings;
                self.settings_revision += 1;
                let result = json!({"settings":self.settings,"settings_revision":self.settings_revision.to_string()});
                self.emit("settings.changed", None, None, result.clone());
                Ok(result)
            }
            _ => unreachable!("command routed to the wrong handler"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn request(method: Method, params: Value) -> Request {
        Request {
            protocol: protocol::PROTOCOL_VERSION.into(),
            request_id: Uuid::new_v4().to_string(),
            method,
            params,
        }
    }

    #[test]
    fn final_render_settings_reject_quality_overrides_and_accept_threads() {
        let mut core = CoreState::new(Limits::default());
        for (samples, depth, message) in [(Some(8), None, "Sampler"), (None, Some(4), "Integrator")]
        {
            let response = core.handle(request(
                Method::RenderSetSettings,
                json!({"expected_settings_revision":"0","settings":{
                    "samples_per_pixel":samples,"threads":null,"max_depth":depth
                }}),
            ));
            assert!(response.error.unwrap().message.contains(message));
        }
        let response = core.handle(request(
            Method::RenderSetSettings,
            json!({"expected_settings_revision":"0","settings":{
                "samples_per_pixel":null,"threads":6,"gpu":true,"max_depth":null
            }}),
        ));
        let settings = response.result.unwrap()["settings"].clone();
        assert_eq!(settings["samples_per_pixel"], Value::Null);
        assert_eq!(settings["threads"], 6);
        assert_eq!(settings["gpu"], true);
        assert_eq!(settings["max_depth"], Value::Null);
    }
}
