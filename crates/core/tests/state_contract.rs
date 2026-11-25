use core::CoreState;
use protocol::commands::Limits;
use protocol::{ErrorCode, Method, Request};
use serde_json::{Value, json};
fn request(id: &str, method: Method, params: Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: id.into(),
        method,
        params,
    }
}
/// A state-changing request that needs no project: bump the render settings.
fn set_threads(core: &mut CoreState, id: &str, threads: u32) -> protocol::Response {
    let revision = core.snapshot().settings_revision;
    core.handle(request(
        id,
        Method::RenderSetSettings,
        json!({"settings":{"samples_per_pixel":null,"threads":threads,"max_depth":null},"expected_settings_revision":revision}),
    ))
}
#[test]
fn test_core_expired_events_require_resync_and_snapshot_cursor_resumes() {
    let limits = Limits {
        retained_events: 2,
        ..Limits::default()
    };
    let mut core = CoreState::new(limits);
    for (index, threads) in [2, 4, 8].into_iter().enumerate() {
        assert!(
            set_threads(&mut core, &format!("set-{index}"), threads)
                .error
                .is_none()
        );
    }
    let response = core.handle(request("expired", Method::EventsRead, json!({"after":"0"})));
    assert_eq!(response.error.unwrap().code, ErrorCode::ResyncRequired);
    let cursor = core.snapshot().event_sequence;
    set_threads(&mut core, "next", 16);
    let response = core.handle(request(
        "events",
        Method::EventsRead,
        json!({"after":cursor,"limit":1}),
    ));
    let result = response.result.unwrap();
    assert_eq!(result["events"].as_array().unwrap().len(), 1);
    assert_eq!(result["events"][0]["kind"], "settings.changed");
    assert_eq!(result["next_cursor"], result["events"][0]["sequence"]);
    let future = core.handle(request(
        "future",
        Method::EventsRead,
        json!({"after":"9999"}),
    ));
    assert_eq!(future.error.unwrap().code, ErrorCode::InvalidCursor);
}
#[test]
fn test_core_request_limit_preserves_existing_replays_and_allows_reads() {
    let mut core = CoreState::new(Limits {
        request_records: 1,
        ..Limits::default()
    });
    let first = set_threads(&mut core, "first", 2);
    assert!(first.error.is_none());
    let busy = set_threads(&mut core, "next", 4);
    assert_eq!(busy.error.unwrap().code, ErrorCode::Busy);
    // The recorded request still replays its original answer.
    let replay = core.handle(request(
        "first",
        Method::RenderSetSettings,
        json!({"settings":{"samples_per_pixel":null,"threads":2,"max_depth":null},"expected_settings_revision":"0"}),
    ));
    assert_eq!(replay, first);
    assert!(
        core.handle(request("state", Method::AppGetState, json!({})))
            .error
            .is_none()
    );
}
#[test]
fn test_core_rejects_unknown_params_and_invalid_revisions() {
    let mut core = CoreState::new(Limits::default());
    let settings = json!({"samples_per_pixel":null,"threads":1,"max_depth":null});
    for params in [
        json!({"settings":settings,"expected_settings_revision":"0","extra":1}),
        json!({"settings":settings,"expected_settings_revision":"00"}),
        json!({"settings":settings,"expected_settings_revision":"18446744073709551616"}),
        json!({"settings":settings}),
    ] {
        assert_eq!(
            core.handle(request(
                &uuid::Uuid::new_v4().to_string(),
                Method::RenderSetSettings,
                params
            ))
            .error
            .unwrap()
            .code,
            ErrorCode::InvalidRequest
        );
    }
    let relative = core.handle(request(
        "relative",
        Method::ProjectOpen,
        json!({"path":"project","expected_current":core.snapshot().project.context}),
    ));
    assert!(relative.error.is_some());
}
#[test]
fn test_core_capabilities_and_protocol_negotiation_are_honest() {
    let mut core = CoreState::new(Limits::default());
    let capabilities = core
        .handle(request("caps", Method::AppGetCapabilities, json!({})))
        .result
        .unwrap();
    assert_eq!(capabilities["render"], false);
    assert_eq!(capabilities["project_management"], true);
    // The retired viewer path reports nothing.
    for retired in ["scene_open", "scene_reload", "scene_index", "load_cancel"] {
        assert!(capabilities.get(retired).is_none(), "{retired}");
    }
    let state = core
        .handle(request("state", Method::AppGetState, json!({})))
        .result
        .unwrap();
    for retired in [
        "scene",
        "scene_revision",
        "view",
        "view_revision",
        "selection",
    ] {
        assert!(state.get(retired).is_none(), "{retired}");
    }
    let response = core.handle(request(
        "hello",
        Method::AppHello,
        json!({"client_name":"test","supported_protocols":["1"]}),
    ));
    assert_eq!(response.error.unwrap().code, ErrorCode::UnsupportedProtocol);
    let mut bad = request("bad", Method::AppGetState, json!({}));
    bad.protocol = "1".into();
    assert_eq!(
        core.handle(bad).error.unwrap().code,
        ErrorCode::UnsupportedProtocol
    );
}
