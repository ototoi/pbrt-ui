use std::io::{BufReader, Cursor};

use protocol::ndjson::{WireError, canonical_operation, decode, read_message, write_message};
use protocol::{ErrorCode, ErrorPayload, Event, MAX_LINE_BYTES, Request, Response};
use serde_json::{Value, json};

fn request(params: Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: "request-1".into(),
        method: protocol::Method::ProjectOpen,
        params,
    }
}

#[test]
fn test_wire_rejects_unknown_envelope_fields_and_enums() {
    for line in [
        r#"{"protocol":"1","request_id":"r","method":"app.hello","extra":true}"#,
        r#"{"protocol":"1","request_id":"r","method":"unknown.method"}"#,
    ] {
        assert!(decode::<Request>(line.as_bytes()).is_err());
        assert!(serde_json::from_str::<Request>(line).is_err());
    }
    assert!(
        decode::<Response>(br#"{"protocol":"1","request_id":"r","result":{},"extra":0}"#).is_err()
    );
    assert!(decode::<Response>(br#"{"protocol":"1","request_id":"r","error":{"code":"io_error","message":"failed","extra":0}}"#).is_err());
    assert!(
        decode::<Response>(
            br#"{"protocol":"1","request_id":"r","error":{"code":"unknown","message":"failed"}}"#
        )
        .is_err()
    );
    assert!(decode::<Event>(br#"{"sequence":"1","kind":"test","extra":0}"#).is_err());
}

#[test]
fn test_wire_rejects_duplicate_keys_recursively_and_after_unescaping() {
    for line in [
        r#"{"protocol":"1","protocol":"1","request_id":"r","method":"app.hello"}"#,
        r#"{"protocol":"1","request_id":"r","method":"app.hello","params":{"a":1,"a":2}}"#,
        r#"{"protocol":"1","request_id":"r","method":"app.hello","params":{"a":[{"x":1,"\u0078":2}]}}"#,
    ] {
        let error = decode::<Request>(line.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("duplicate key"), "{error}");
    }
}

#[test]
fn test_wire_response_requires_exactly_one_result_or_error() {
    for fields in [
        "",
        r#", "error":null"#,
        r#", "result":{}, "error":null"#,
        r#", "result":null, "error":{"code":"io_error","message":"failed"}"#,
    ] {
        let line = format!(r#"{{"protocol":"1","request_id":"r"{fields}}}"#);
        assert!(decode::<Response>(line.as_bytes()).is_err(), "{line}");
        assert!(serde_json::from_str::<Response>(&line).is_err(), "{line}");
    }
    let response: Response = decode(br#"{"protocol":"1","request_id":"r","result":null}"#).unwrap();
    assert_eq!(response.result, Some(Value::Null));
    assert_eq!(response.error, None);
    let failure = Response::err("r", ErrorPayload::new(ErrorCode::Busy, "queue full"));
    let mut bytes = Vec::new();
    write_message(&mut bytes, &failure).unwrap();
    assert_eq!(decode::<Response>(&bytes).unwrap(), failure);
}

#[test]
fn test_wire_roundtrips_multiple_frames_across_small_reads() {
    let expected = request(json!({"absolute_path":"/tmp/日本語.pbrt", "text":"line\nline"}));
    let mut bytes = Vec::new();
    write_message(&mut bytes, &expected).unwrap();
    bytes.extend_from_slice(
        b"{\"protocol\":\"1\",\"request_id\":\"r\",\"method\":\"job.list\"}\r\n",
    );
    let mut reader = BufReader::with_capacity(3, Cursor::new(bytes));
    assert_eq!(
        read_message::<Request>(&mut reader).unwrap(),
        Some(expected)
    );
    assert_eq!(
        read_message::<Request>(&mut reader)
            .unwrap()
            .unwrap()
            .method,
        protocol::Method::JobList
    );
    assert!(read_message::<Request>(&mut reader).unwrap().is_none());
}

#[test]
fn test_wire_rejects_truncated_invalid_utf8_and_multiline_frames() {
    let mut reader = Cursor::new(br#"{"protocol":"1"}"#);
    assert!(matches!(
        read_message::<Request>(&mut reader),
        Err(WireError::UnterminatedLine)
    ));
    for bytes in [
        b"\xff".as_slice(),
        b"{} {}",
        b"{\n}",
        b"{}\r",
        b"{}\n\n",
        b"[]",
        b"null",
        b"",
    ] {
        assert!(decode::<Value>(bytes).is_err(), "{bytes:?}");
    }
}

#[test]
fn test_wire_enforces_exact_byte_limit_for_lf_and_crlf() {
    let mut payload = Vec::with_capacity(MAX_LINE_BYTES + 2);
    payload.extend_from_slice(b"{\"x\":\"");
    payload.resize(MAX_LINE_BYTES - 2, b'a');
    payload.extend_from_slice(b"\"}");
    assert_eq!(payload.len(), MAX_LINE_BYTES);
    assert!(decode::<Value>(&payload).is_ok());
    for ending in [b"\n".as_slice(), b"\r\n"] {
        let mut frame = payload.clone();
        frame.extend_from_slice(ending);
        let mut reader = BufReader::with_capacity(4096, Cursor::new(frame));
        assert!(read_message::<Value>(&mut reader).unwrap().is_some());
    }
    payload.insert(payload.len() - 2, b'a');
    assert!(matches!(
        decode::<Value>(&payload),
        Err(WireError::LineTooLong)
    ));
    payload.push(b'\n');
    assert!(matches!(
        read_message::<Value>(&mut Cursor::new(payload)),
        Err(WireError::LineTooLong)
    ));
}

#[test]
fn test_wire_stops_reading_unbounded_input_at_the_limit() {
    let mut reader = BufReader::with_capacity(1024, std::io::repeat(b' '));
    assert!(matches!(
        read_message::<Request>(&mut reader),
        Err(WireError::LineTooLong)
    ));
}

#[test]
fn test_wire_writes_nothing_for_invalid_or_oversized_messages() {
    let invalid = Response {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: "r".into(),
        result: None,
        error: None,
    };
    let mut output = Vec::new();
    assert!(write_message(&mut output, &invalid).is_err());
    assert!(output.is_empty());
    let oversized = request(json!({"text":"a".repeat(MAX_LINE_BYTES)}));
    assert!(matches!(
        write_message(&mut output, &oversized),
        Err(WireError::LineTooLong)
    ));
    assert!(output.is_empty());
}

#[test]
fn test_wire_canonical_operation_sorts_objects_and_preserves_arrays() {
    let left = request(serde_json::from_str(r#"{"z":[{"β":2,"a":1},3],"a":null}"#).unwrap());
    let mut right = request(serde_json::from_str(r#"{"a":null,"z":[{"a":1,"β":2},3]}"#).unwrap());
    right.request_id = "another-request".into();
    assert_eq!(
        canonical_operation(&left).unwrap(),
        canonical_operation(&right).unwrap()
    );
    assert_eq!(
        String::from_utf8(canonical_operation(&left).unwrap()).unwrap(),
        r#"{"method":"project.open","params":{"a":null,"z":[{"a":1,"β":2},3]}}"#
    );
    right.params["z"] = json!([3, {"a":1,"β":2}]);
    assert_ne!(
        canonical_operation(&left).unwrap(),
        canonical_operation(&right).unwrap()
    );
    right = left.clone();
    right.method = protocol::Method::ProjectClose;
    assert_ne!(
        canonical_operation(&left).unwrap(),
        canonical_operation(&right).unwrap()
    );
}

#[test]
fn test_ipr_methods_and_params_round_trip() {
    use protocol::jobs::{IprSetParams, IprSettings, IprStatus, IprSubmitParams};
    let line = r#"{"protocol":"1","request_id":"r","method":"render.ipr_set","params":{"enabled":true,"settings":{"samples_per_pixel":8,"threads":1,"max_depth":null,"debounce_ms":400}}}"#;
    let parsed = decode::<Request>(line.as_bytes()).unwrap();
    assert_eq!(parsed.method, protocol::Method::RenderIprSet);
    let params: IprSetParams = serde_json::from_value(parsed.params).unwrap();
    assert_eq!(
        params.settings,
        IprSettings {
            samples_per_pixel: 8,
            threads: Some(1),
            max_depth: None,
            debounce_ms: 400
        }
    );
    assert!(
        decode::<Request>(
            br#"{"protocol":"1","request_id":"r","method":"render.ipr_submit","params":{}}"#
        )
        .is_ok()
    );
    let submit: IprSubmitParams = serde_json::from_str(r#"{"project":null,"view":null}"#).unwrap();
    assert_eq!(submit.project, None);
    assert_eq!(submit.view, None);
    let implicit: IprSetParams = serde_json::from_str(
        r#"{"enabled":true,"settings":{"samples_per_pixel":8,"threads":1,"debounce_ms":400}}"#,
    )
    .unwrap();
    assert_eq!(implicit.settings.max_depth, None);
    assert!(
        serde_json::from_str::<IprSetParams>(
            r#"{"enabled":true,"settings":{"samples_per_pixel":8,"threads":1,"max_depth":null,"debounce_ms":400,"extra":1}}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<IprSetParams>(r#"{"settings":{}}"#).is_err());
    assert!(
        serde_json::from_str::<IprSubmitParams>(r#"{"projects":{}}"#).is_err(),
        "unknown submit field"
    );
    let status = IprStatus {
        enabled: true,
        settings: IprSettings::default(),
        active_job_id: None,
        waiting_for_final_render: false,
    };
    let encoded = serde_json::to_string(&status).unwrap();
    let decoded: IprStatus = serde_json::from_str(&encoded).unwrap();
    assert!(decoded.enabled);
    assert_eq!(decoded.active_job_id, None);
}

#[test]
fn test_ipr_requests_are_deduplicated_by_canonical_operation() {
    let base = Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: "r".into(),
        method: protocol::Method::RenderIprSet,
        params: json!({"enabled":true,"settings":{"samples_per_pixel":8,"threads":1,"max_depth":null,"debounce_ms":400}}),
    };
    let mut same = base.clone();
    same.request_id = "other".into();
    same.params["settings"] = serde_json::from_str(
        r#"{"max_depth":null,"debounce_ms":400,"threads":1,"samples_per_pixel":8}"#,
    )
    .unwrap();
    assert_eq!(
        canonical_operation(&base).unwrap(),
        canonical_operation(&same).unwrap()
    );
    let mut louder = base.clone();
    louder.params["settings"]["samples_per_pixel"] = json!(64);
    assert_ne!(
        canonical_operation(&base).unwrap(),
        canonical_operation(&louder).unwrap()
    );
}

#[test]
fn test_wire_accepts_project_preview_method() {
    // Positive registration pin: "project.preview" must not decode as unknown.
    let request: Request = serde_json::from_str(
        r#"{"protocol":"1","request_id":"r","method":"project.preview","params":{"project":{"project_id":"00000000-0000-0000-0000-000000000000","activation":"1","revision":"1"},"destination":"/tmp/slot"}}"#,
    )
    .unwrap();
    assert_eq!(request.method, protocol::Method::ProjectPreview);
    let params: protocol::project::PreviewProjectParams =
        serde_json::from_value(request.params).unwrap();
    assert_eq!(params.destination, "/tmp/slot");
    assert!(
        params.stage_assets,
        "external callers keep the self-contained bundle by default"
    );
}

#[test]
fn test_wire_accepts_project_preview_without_staged_assets() {
    // The desktop preview slot can opt out of staging assets, while callers
    // that omit the field retain the default behavior.
    let request: Request = serde_json::from_str(
        r#"{"protocol":"1","request_id":"r","method":"project.preview","params":{"project":{"project_id":"00000000-0000-0000-0000-000000000000","activation":"1","revision":"1"},"destination":"/tmp/slot","stage_assets":false}}"#,
    )
    .unwrap();
    assert_eq!(request.method, protocol::Method::ProjectPreview);
    let params: protocol::project::PreviewProjectParams =
        serde_json::from_value(request.params).unwrap();
    assert!(!params.stage_assets);
}

#[test]
fn test_wire_accepts_optional_preview_content_store() {
    let request: Request = serde_json::from_str(
        r#"{"protocol":"1","request_id":"r","method":"project.preview","params":{"project":{"project_id":"00000000-0000-0000-0000-000000000000","activation":"1","revision":"1"},"destination":"/tmp/slot","store":"/tmp/store"}}"#,
    )
    .unwrap();
    let params: protocol::project::PreviewProjectParams =
        serde_json::from_value(request.params).unwrap();
    assert_eq!(params.store.as_deref(), Some("/tmp/store"));
}

#[test]
fn test_wire_accepts_project_spectrum_color_and_rejects_unknown_fields() {
    let request: Request = serde_json::from_str(
        r#"{"protocol":"1","request_id":"r","method":"project.spectrum_color","params":{"project":{"project_id":"00000000-0000-0000-0000-000000000000","activation":"1","revision":"1"},"target":{"kind":"resource","id":"00000000-0000-0000-0000-000000000011"},"parameter":"L"}}"#,
    )
    .unwrap();
    assert_eq!(request.method, protocol::Method::ProjectSpectrumColor);
    let params: protocol::project::SpectrumColorParams =
        serde_json::from_value(request.params).unwrap();
    match params.target {
        protocol::project::ParameterTarget::Resource(id) => assert_eq!(
            id,
            "00000000-0000-0000-0000-000000000011"
                .parse::<uuid::Uuid>()
                .unwrap()
        ),
        other => panic!("wrong target: {other:?}"),
    }
    assert_eq!(params.parameter, "L");
    assert_eq!(params.role, None);
    assert!(
        serde_json::from_value::<protocol::project::SpectrumColorParams>(serde_json::json!({
            "project":{"project_id":"00000000-0000-0000-0000-000000000000","activation":"1","revision":"1"},
            "target":{"kind":"resource","id":"00000000-0000-0000-0000-000000000011"},
            "parameter":"L","bogus":true
        }))
        .is_err(),
        "the params keep deny_unknown_fields"
    );
}
