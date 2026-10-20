//! Unit tests for the tool protocol layer.
//!
//! These tests are the Rust port of `tests/unit/test_protocol_branches.py`
//! and cover the parts of the public surface that the live differential
//! cannot reach: the typed construction of an envelope, the worker bootstrap,
//! the child environment, the request and response streams and the workspace
//! rules on a real file system.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kala_contracts::{ContractModel, ToolCall, ToolDescriptor, ToolRunStatus};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::protocol::{
    MAX_JSON_BYTES, PROTECTED_CHILD_ENVIRONMENT_KEYS, PROTOCOL_VERSION, SubprocessToolHandler,
    ToolEnvelopeKind, ToolErrorCode, ToolHandler, ToolProtocolError, ToolRequestEnvelope,
    ToolResponseEnvelope, build_child_environment, build_worker_command, canonical_json_digest,
    canonical_json_dumps, check_object_schema, read_request_stream, resolve_workspace_path,
    sanitize_child_stderr, sha256_hex, strict_json_loads, summarize_error, validate_against_schema,
    write_response_stream,
};
use num_bigint::BigInt;

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::object(entries)
}

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::text(value)
}

fn integer(value: i64) -> CanonicalValue {
    CanonicalValue::Integer(BigInt::from(value))
}

fn arguments() -> CanonicalObject {
    let mut entries = CanonicalObject::new();
    entries.insert(JsonText::new("operation"), text("beam_volume"));
    entries
}

fn request() -> ToolRequestEnvelope {
    ToolRequestEnvelope::new(
        PROTOCOL_VERSION,
        "call-1",
        "mechanical.reference-simulator",
        "1.0.0",
        &arguments(),
        "/tmp/workspace",
        30.0,
        &BigInt::from(1_000_000),
        &BigInt::from(1_000_000),
        &[],
    )
    .expect("request envelope")
}

fn descriptor() -> ToolDescriptor {
    let mut entries = CanonicalObject::new();
    entries.insert(
        JsonText::new("tool_id"),
        text("mechanical.reference-simulator"),
    );
    entries.insert(JsonText::new("version"), text("1.0.0"));
    entries.insert(JsonText::new("name"), text("Reference simulator"));
    entries.insert(JsonText::new("description"), text("A reference simulator."));
    entries.insert(JsonText::new("input_schema_digest"), text(&"a".repeat(64)));
    entries.insert(JsonText::new("output_schema_digest"), text(&"b".repeat(64)));
    entries.insert(JsonText::new("side_effect_level"), text("none"));
    ToolDescriptor::from_value(&CanonicalValue::Object(entries)).expect("descriptor")
}

// ---------------------------------------------------------------------------
// The error hierarchy.
// ---------------------------------------------------------------------------

#[test]
fn the_error_classes_match_the_python_hierarchy() {
    let error = ToolProtocolError::json_envelope("payload is not strict JSON");
    assert_eq!(error.python_class_name(), "JsonEnvelopeError");
    assert!(error.is_json_envelope());
    assert!(!error.is_envelope());
    assert_eq!(error.to_string(), "payload is not strict JSON");

    let error = ToolProtocolError::envelope("call_id must be a safe identifier");
    assert_eq!(error.python_class_name(), "ToolEnvelopeError");

    let error = ToolProtocolError::workspace("workspace paths must be relative");
    assert_eq!(error.python_class_name(), "ToolWorkspaceError");

    let error = ToolProtocolError::protocol("no python interpreter");
    assert_eq!(error.python_class_name(), "ToolProtocolError");

    let error = ToolProtocolError::schema("object keys must be strings", "$.a");
    assert_eq!(error.python_class_name(), "SchemaValidationError");
    assert_eq!(error.to_string(), "$.a: object keys must be strings");
    assert_eq!(error.path(), Some("$.a"));
    assert_eq!(error.reason(), Some("object keys must be strings"));
}

// ---------------------------------------------------------------------------
// Canonical JSON.
// ---------------------------------------------------------------------------

#[test]
fn canonical_json_sorts_and_escapes() {
    let value = object(vec![("b", integer(1)), ("a", text("é"))]);
    assert_eq!(
        canonical_json_dumps(&value, MAX_JSON_BYTES).expect("canonical"),
        "{\"a\":\"\\u00e9\",\"b\":1}"
    );
}

#[test]
fn canonical_json_enforces_the_byte_budget() {
    let value = text(&"x".repeat(100));
    let error = canonical_json_dumps(&value, 10).expect_err("budget");
    assert_eq!(error.message(), "canonical JSON exceeds the bounded size");
}

#[test]
fn strict_decoding_refuses_duplicates_and_constants() {
    let error = strict_json_loads(br#"{"a":1,"a":2}"#, MAX_JSON_BYTES).expect_err("duplicates");
    assert_eq!(
        error.message(),
        "duplicate JSON object keys are not permitted"
    );
    let error = strict_json_loads(b"NaN", MAX_JSON_BYTES).expect_err("constant");
    assert_eq!(error.message(), "JSON constant NaN is not permitted");
    let error = strict_json_loads(b"1e400", MAX_JSON_BYTES).expect_err("infinite");
    assert_eq!(error.message(), "JSON numbers must be finite");
    let error = strict_json_loads(b"{not json", MAX_JSON_BYTES).expect_err("malformed");
    assert_eq!(error.message(), "payload is not strict JSON");
    let error = strict_json_loads(b"\xff\xfe", MAX_JSON_BYTES).expect_err("utf8");
    assert_eq!(error.message(), "payload is not valid UTF-8");
    let error = strict_json_loads(b"12345", 2).expect_err("budget");
    assert_eq!(error.message(), "payload exceeds the bounded size");
}

#[test]
fn digests_are_stable_across_key_order() {
    let left = object(vec![("a", integer(1)), ("b", integer(2))]);
    let right = object(vec![("b", integer(2)), ("a", integer(1))]);
    assert_eq!(
        canonical_json_digest(&left).expect("digest"),
        canonical_json_digest(&right).expect("digest")
    );
    assert_eq!(sha256_hex(b"payload"), sha256_hex("payload".as_bytes()));
}

// ---------------------------------------------------------------------------
// The request envelope.
// ---------------------------------------------------------------------------

#[test]
fn a_request_envelope_binds_its_exact_payload() {
    let request = request();
    assert_eq!(request.kind(), ToolEnvelopeKind::Request);
    assert_eq!(request.arguments_json(), r#"{"operation":"beam_volume"}"#);
    assert_eq!(
        request.arguments_digest(),
        sha256_hex(request.arguments_json().as_bytes())
    );
    let payload = request.to_payload();
    let CanonicalValue::Object(entries) = &payload else {
        panic!("payload is not an object")
    };
    assert_eq!(
        entries
            .get(&JsonText::new("kind"))
            .and_then(|value| value.as_text()),
        Some(&JsonText::new("request"))
    );
    assert_eq!(
        request.envelope_digest().expect("digest"),
        canonical_json_digest(&payload).expect("digest")
    );
    let json = request.to_json().expect("json");
    assert_eq!(
        ToolRequestEnvelope::from_json(json.as_bytes()).expect("round trip"),
        request
    );
}

#[test]
fn a_request_envelope_refuses_an_unsafe_workspace() {
    let error = ToolRequestEnvelope::new(
        PROTOCOL_VERSION,
        "call-1",
        "tool",
        "1.0.0",
        &arguments(),
        "relative/workspace",
        30.0,
        &BigInt::from(1),
        &BigInt::from(1),
        &[],
    )
    .expect_err("workspace");
    assert_eq!(error.message(), "workspace must be an absolute path");
}

#[test]
fn a_request_envelope_refuses_bad_numeric_limits() {
    let error = ToolRequestEnvelope::new(
        PROTOCOL_VERSION,
        "call-1",
        "tool",
        "1.0.0",
        &arguments(),
        "/tmp/workspace",
        0.0,
        &BigInt::from(1),
        &BigInt::from(1),
        &[],
    )
    .expect_err("deadline");
    assert!(error.message().contains("deadline_seconds"));

    let error = ToolRequestEnvelope::new(
        PROTOCOL_VERSION,
        "call-1",
        "tool",
        "1.0.0",
        &arguments(),
        "/tmp/workspace",
        1.0,
        &BigInt::from(0),
        &BigInt::from(1),
        &[],
    )
    .expect_err("output budget");
    assert!(error.message().contains("max_output_bytes"));
}

#[test]
fn a_request_envelope_refuses_an_unsafe_identifier() {
    let error = ToolRequestEnvelope::new(
        PROTOCOL_VERSION,
        "has space",
        "tool",
        "1.0.0",
        &arguments(),
        "/tmp/workspace",
        1.0,
        &BigInt::from(1),
        &BigInt::from(1),
        &[],
    )
    .expect_err("identifier");
    assert_eq!(error.message(), "call_id must be a safe identifier");
}

#[test]
fn a_request_envelope_sorts_and_deduplicates_capabilities() {
    let request = ToolRequestEnvelope::new(
        PROTOCOL_VERSION,
        "call-1",
        "tool",
        "1.0.0",
        &arguments(),
        "/tmp/workspace",
        1.0,
        &BigInt::from(1),
        &BigInt::from(1),
        &[
            "artifact.write".to_owned(),
            "artifact.read".to_owned(),
            "artifact.read".to_owned(),
        ],
    )
    .expect("request");
    assert_eq!(
        request.granted_capabilities(),
        ["artifact.read".to_owned(), "artifact.write".to_owned()]
    );
}

#[test]
fn a_request_envelope_is_built_from_a_tool_call() {
    let mut entries = CanonicalObject::new();
    entries.insert(JsonText::new("call_id"), text("call-9"));
    entries.insert(
        JsonText::new("tool_id"),
        text("mechanical.reference-simulator"),
    );
    entries.insert(JsonText::new("tool_version"), text("1.0.0"));
    entries.insert(
        JsonText::new("arguments_json"),
        text(r#"{"operation":"bar_summary"}"#),
    );
    entries.insert(JsonText::new("side_effect_level"), text("none"));
    entries.insert(JsonText::new("timeout_seconds"), CanonicalValue::Float(5.0));
    entries.insert(
        JsonText::new("issued_at"),
        text("2026-01-02T03:04:05+00:00"),
    );
    let call = ToolCall::from_value(&CanonicalValue::Object(entries)).expect("tool call");
    let directory = tempfile::tempdir().expect("temporary directory");
    let request = ToolRequestEnvelope::from_call(
        &call,
        directory.path(),
        2.0,
        &BigInt::from(10),
        &BigInt::from(10),
        &["artifact.read".to_owned(), "artifact.read".to_owned()],
    )
    .expect("request");
    assert_eq!(request.call_id(), "call-9");
    assert_eq!(request.arguments_json(), r#"{"operation":"bar_summary"}"#);
    assert_eq!(request.granted_capabilities(), ["artifact.read".to_owned()]);
}

// ---------------------------------------------------------------------------
// The response envelope.
// ---------------------------------------------------------------------------

#[test]
fn a_successful_response_carries_its_result() {
    let mut result = CanonicalObject::new();
    result.insert(JsonText::new("ok"), CanonicalValue::Bool(true));
    let envelope =
        ToolResponseEnvelope::succeeded_envelope("call-1", "tool", "1.0.0", &result, &[], 0.0)
            .expect("response");
    assert!(envelope.succeeded());
    assert_eq!(
        envelope.result_json().expect("json").as_deref(),
        Some(r#"{"ok":true}"#)
    );
    assert_eq!(
        envelope.result_digest().expect("digest"),
        sha256_hex(br#"{"ok":true}"#)
    );
    let json = envelope.to_json().expect("json");
    assert_eq!(
        ToolResponseEnvelope::from_json(json.as_bytes()).expect("round trip"),
        envelope
    );
}

#[test]
fn a_failed_response_digests_the_empty_object() {
    let envelope = ToolResponseEnvelope::failed_envelope_with_code(
        "call-1",
        "tool",
        "1.0.0",
        ToolRunStatus::Failed,
        ToolErrorCode::InternalError,
        Some("the tool failed"),
        0.0,
    )
    .expect("response");
    assert!(!envelope.succeeded());
    assert_eq!(envelope.result_json().expect("json"), None);
    assert_eq!(envelope.result_digest().expect("digest"), sha256_hex(b"{}"));
    assert_eq!(envelope.error_code(), Some("tool_error"));
}

#[test]
fn a_failure_needs_an_error_code() {
    let error = ToolResponseEnvelope::failed_envelope(
        "call-1",
        "tool",
        "1.0.0",
        ToolRunStatus::Failed,
        None,
        None,
        0.0,
    )
    .expect_err("error code");
    assert_eq!(error.message(), "failed tool runs require an error code");
}

#[test]
fn a_failure_cannot_carry_a_result() {
    let mut result = CanonicalObject::new();
    result.insert(JsonText::new("ok"), CanonicalValue::Bool(true));
    let error = ToolResponseEnvelope::new(
        PROTOCOL_VERSION,
        "call-1",
        "tool",
        "1.0.0",
        ToolRunStatus::Failed,
        Some(&result),
        Some("tool_error"),
        Some("failed"),
        &[],
        0.0,
    )
    .expect_err("result");
    assert_eq!(
        error.message(),
        "failed tool runs cannot carry a result payload"
    );
}

#[test]
fn a_success_needs_a_result() {
    let error = ToolResponseEnvelope::new(
        PROTOCOL_VERSION,
        "call-1",
        "tool",
        "1.0.0",
        ToolRunStatus::Succeeded,
        None,
        None,
        None,
        &[],
        0.0,
    )
    .expect_err("result");
    assert_eq!(
        error.message(),
        "successful tool runs require a result payload"
    );
}

#[test]
fn a_duration_must_be_finite_and_nonnegative() {
    let mut result = CanonicalObject::new();
    result.insert(JsonText::new("ok"), CanonicalValue::Bool(true));
    let error = ToolResponseEnvelope::new(
        PROTOCOL_VERSION,
        "call-1",
        "tool",
        "1.0.0",
        ToolRunStatus::Succeeded,
        Some(&result),
        None,
        None,
        &[],
        -1.0,
    )
    .expect_err("duration");
    assert_eq!(
        error.message(),
        "duration_ms must be finite and nonnegative"
    );
}

#[test]
fn a_response_records_the_call() {
    let mut result = CanonicalObject::new();
    result.insert(JsonText::new("ok"), CanonicalValue::Bool(true));
    let envelope =
        ToolResponseEnvelope::succeeded_envelope("call-1", "tool", "1.0.0", &result, &[], 2.0)
            .expect("response");
    let started = kala_contracts::Timestamp::new(2026, 1, 2, 3, 4, 5, 0, 0).expect("timestamp");
    let finished = kala_contracts::Timestamp::new(2026, 1, 2, 3, 4, 6, 0, 0).expect("timestamp");
    let recorded = envelope.to_result(&started, &finished).expect("result");
    assert_eq!(recorded.call_id(), "call-1");
    assert_eq!(recorded.status(), ToolRunStatus::Succeeded);
    assert_eq!(recorded.duration_ms(), Some(2.0));

    let error = envelope
        .to_result(&finished, &started)
        .expect_err("ordering");
    assert_eq!(error.message(), "finished_at cannot precede started_at");
}

// ---------------------------------------------------------------------------
// The worker bootstrap and the child environment.
// ---------------------------------------------------------------------------

#[test]
fn the_bootstrap_is_the_python_module_bootstrap() {
    let bootstrap = kala_tools::protocol::WORKER_BOOTSTRAP;
    assert!(bootstrap.starts_with("import importlib\nimport sys\n"));
    assert!(bootstrap.contains("from kala_agent.tools.protocol import ("));
    assert!(bootstrap.contains("read_request_stream(sys.stdin.buffer)"));
    assert!(bootstrap.contains("write_response_stream(envelope, channel)"));
    assert!(!bootstrap.ends_with('\n'));
}

#[test]
fn a_subprocess_handler_validates_its_entry_point() {
    let handler = SubprocessToolHandler::new(
        descriptor(),
        "kala_agent.tools.simulator:invoke",
        &["-B", "-s"],
    )
    .expect("handler");
    assert_eq!(handler.entry_point(), "kala_agent.tools.simulator:invoke");
    assert_eq!(
        handler.interpreter_args(),
        ["-B".to_owned(), "-s".to_owned()]
    );
    assert_eq!(
        handler.descriptor().tool_id(),
        "mechanical.reference-simulator"
    );

    for entry_point in ["no-separator", ":invoke", "module:", "1module:invoke"] {
        assert!(
            SubprocessToolHandler::new(descriptor(), entry_point, &[]).is_err(),
            "entry point {entry_point:?} was accepted"
        );
    }
}

#[test]
fn a_subprocess_handler_refuses_bootstrap_replacing_flags() {
    for flag in ["-c", "-E", "-I", "-m", "not-a-flag"] {
        assert!(
            SubprocessToolHandler::new(descriptor(), "module:invoke", &[flag]).is_err(),
            "interpreter argument {flag:?} was accepted"
        );
    }
}

#[test]
fn a_subprocess_handler_cannot_run_in_process() {
    let handler = SubprocessToolHandler::new(descriptor(), "module:invoke", &[]).expect("handler");
    let error = handler.invoke(&request()).expect_err("in process");
    assert_eq!(error.python_class_name(), "ToolProtocolError");
    assert!(error.message().contains("dispatched by the executor"));
}

#[test]
fn the_worker_command_names_the_bootstrap() {
    let handler =
        SubprocessToolHandler::new(descriptor(), "module:invoke", &["-B"]).expect("handler");
    let command = build_worker_command(&handler, Some("/usr/bin/python3")).expect("command");
    assert_eq!(command[0], "/usr/bin/python3");
    assert!(command.contains(&"-c".to_owned()));
    assert!(command.contains(&kala_tools::protocol::WORKER_BOOTSTRAP.to_owned()));
    assert_eq!(command.last().expect("entry point"), "module:invoke");

    let error = build_worker_command(&handler, None).expect_err("interpreter");
    assert!(error.message().contains("no python interpreter"));
    let error = build_worker_command(&handler, Some("")).expect_err("interpreter");
    assert!(error.message().contains("no python interpreter"));
}

#[test]
fn the_child_environment_pins_every_protected_key() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let roots = PathBuf::from("/opt/roots");
    let mut extra = BTreeMap::new();
    extra.insert("EXTRA_FLAG".to_owned(), "1".to_owned());
    let environment = build_child_environment(directory.path(), &[roots.as_path()], Some(&extra))
        .expect("environment");
    let workspace = directory.path().to_string_lossy().replace('\\', "/");
    assert_eq!(environment.get("HOME"), Some(&workspace));
    assert_eq!(environment.get("TMPDIR"), Some(&workspace));
    assert_eq!(
        environment.get("PYTHONPATH"),
        Some(&"/opt/roots".to_owned())
    );
    assert_eq!(
        environment.get("KALA_TOOL_PROTOCOL"),
        Some(&PROTOCOL_VERSION.to_owned())
    );
    assert_eq!(environment.get("EXTRA_FLAG"), Some(&"1".to_owned()));
    assert_eq!(
        environment.get("PATH"),
        Some(&"/usr/local/bin:/usr/bin:/bin".to_owned())
    );
}

#[test]
fn the_child_environment_refuses_a_protected_override() {
    let directory = tempfile::tempdir().expect("temporary directory");
    for key in ["HOME", "PATH", "PYTHONPATH", "TMPDIR"] {
        let mut extra = BTreeMap::new();
        extra.insert(key.to_owned(), "/elsewhere".to_owned());
        let error =
            build_child_environment(directory.path(), &[], Some(&extra)).expect_err("protected");
        assert!(error.message().contains("protected by the executor"));
    }
}

#[test]
fn the_child_environment_refuses_a_malformed_override() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let mut extra = BTreeMap::new();
    extra.insert("has space".to_owned(), "1".to_owned());
    let error = build_child_environment(directory.path(), &[], Some(&extra)).expect_err("name");
    assert!(error.message().contains("simple names"));

    let mut extra = BTreeMap::new();
    extra.insert("EXTRA".to_owned(), "a\u{0}b".to_owned());
    let error = build_child_environment(directory.path(), &[], Some(&extra)).expect_err("value");
    assert!(error.message().contains("null-free strings"));
}

#[test]
fn the_protected_keys_are_the_documented_ones() {
    for key in [
        "HOME",
        "KALA_TOOL_PROTOCOL",
        "LANG",
        "PATH",
        "PYTHONDONTWRITEBYTECODE",
        "PYTHONHASHSEED",
        "PYTHONIOENCODING",
        "PYTHONPATH",
        "PYTHONNOUSERSITE",
        "PYTHONSAFEPATH",
        "TMP",
        "TMPDIR",
        "TEMP",
    ] {
        assert!(
            PROTECTED_CHILD_ENVIRONMENT_KEYS.contains(&key),
            "key {key} is not protected"
        );
    }
}

// ---------------------------------------------------------------------------
// The request and response streams.
// ---------------------------------------------------------------------------

#[test]
fn a_request_is_read_from_a_stream() {
    let request = request();
    let mut stream = std::io::Cursor::new(request.to_json().expect("json").into_bytes());
    let decoded = read_request_stream(&mut stream).expect("request");
    assert_eq!(decoded, request);
}

#[test]
fn an_oversized_request_is_refused() {
    let mut stream = std::io::Cursor::new(vec![b'x'; MAX_JSON_BYTES + 1]);
    let error = read_request_stream(&mut stream).expect_err("bounded size");
    assert_eq!(error.message(), "request envelope exceeds the bounded size");
}

#[test]
fn a_response_is_written_to_a_stream() {
    let mut result = CanonicalObject::new();
    result.insert(JsonText::new("ok"), CanonicalValue::Bool(true));
    let envelope =
        ToolResponseEnvelope::succeeded_envelope("call-1", "tool", "1.0.0", &result, &[], 0.0)
            .expect("response");
    let mut stream: Vec<u8> = Vec::new();
    write_response_stream(&envelope, &mut stream).expect("write");
    let written = String::from_utf8(stream).expect("utf-8");
    assert!(written.ends_with('\n'));
    assert_eq!(written, format!("{}\n", envelope.to_json().expect("json")));
}

// ---------------------------------------------------------------------------
// Workspace paths.
// ---------------------------------------------------------------------------

#[test]
fn a_workspace_path_stays_inside_the_run_directory() {
    let directory = tempfile::tempdir().expect("temporary directory");
    std::fs::create_dir(directory.path().join("outputs")).expect("directory");
    let resolved =
        resolve_workspace_path(directory.path(), "./outputs/./report.json").expect("path");
    assert!(resolved.ends_with("outputs/report.json"));
}

#[test]
fn a_workspace_path_refuses_to_leave_the_run_directory() {
    let directory = tempfile::tempdir().expect("temporary directory");
    for relative in ["../escape", "a/../b", "/etc/passwd", ".."] {
        assert!(
            resolve_workspace_path(directory.path(), relative).is_err(),
            "path {relative:?} was accepted"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_workspace_path_refuses_a_symlink() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let real = directory.path().join("real");
    std::fs::create_dir(&real).expect("directory");
    std::os::unix::fs::symlink(&real, directory.path().join("link")).expect("symlink");
    let error = resolve_workspace_path(directory.path(), "link/report.json").expect_err("symlink");
    assert_eq!(error.message(), "workspace paths cannot traverse symlinks");
}

#[cfg(not(unix))]
#[test]
fn a_workspace_path_refuses_a_symlink() {
    // The rule needs a symlink, which this platform does not create here.
}

#[test]
fn a_workspace_path_needs_an_existing_directory() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let error = resolve_workspace_path(&directory.path().join("absent"), "a").expect_err("absent");
    assert_eq!(error.message(), "workspace directory does not exist");
}

#[test]
fn a_workspace_path_refuses_unsafe_segments() {
    let directory = tempfile::tempdir().expect("temporary directory");
    for relative in ["has space", "a\\b", "a\u{0}b"] {
        assert!(
            resolve_workspace_path(directory.path(), relative).is_err(),
            "path {relative:?} was accepted"
        );
    }
}

// ---------------------------------------------------------------------------
// Diagnostics.
// ---------------------------------------------------------------------------

#[test]
fn a_summary_reports_the_class_and_the_message() {
    let error = ToolProtocolError::envelope("call_id must be a safe identifier");
    assert_eq!(
        summarize_error(&error, None),
        "ToolEnvelopeError: call_id must be a safe identifier"
    );
    let error = ToolProtocolError::workspace("failed in /tmp/ws/x");
    assert_eq!(
        summarize_error(&error, Some("/tmp/ws")),
        "ToolWorkspaceError: failed in <workspace>/x"
    );
}

#[test]
fn a_summary_omits_an_empty_message() {
    let error = ToolProtocolError::protocol("");
    assert_eq!(summarize_error(&error, None), "ToolProtocolError");
}

#[test]
fn a_child_stderr_keeps_the_first_and_the_last_line() {
    let payload = b"Traceback (most recent call last):\n  File x\nValueError: boom";
    assert_eq!(
        sanitize_child_stderr(payload, None),
        "Traceback (most recent call last): | ... | ValueError: boom"
    );
    assert_eq!(
        sanitize_child_stderr(b"first\nsecond", None),
        "first | second"
    );
    assert_eq!(
        sanitize_child_stderr(b"first\nsecond\nthird\nfourth\nfifth", None),
        "first | ... | fifth"
    );
    assert_eq!(sanitize_child_stderr(b"", None), "");
    assert_eq!(sanitize_child_stderr(b"\x00\x01only", None), "only");
    assert_eq!(sanitize_child_stderr(b"a\r\nb", None), "a | b");
    assert_eq!(sanitize_child_stderr(b"  \n \n ", None), "");
}

#[test]
fn a_child_stderr_redacts_the_workspace() {
    assert_eq!(
        sanitize_child_stderr(b"failed in /tmp/ws-1234", Some("/tmp/ws-1234")),
        "failed in <workspace>"
    );
    assert_eq!(
        sanitize_child_stderr(b"failed in /tmp/ws", Some("")),
        "failed in /tmp/ws"
    );
}

#[test]
fn a_child_stderr_is_bounded() {
    let payload = vec![b'x'; 4_000];
    assert!(sanitize_child_stderr(&payload, None).len() <= 512);
}

// ---------------------------------------------------------------------------
// Schema admission.
// ---------------------------------------------------------------------------

#[test]
fn an_object_schema_must_be_closed() {
    let schema = object(vec![
        ("type", text("object")),
        ("additionalProperties", CanonicalValue::Bool(false)),
        ("properties", object(vec![("a", object(vec![]))])),
        ("required", CanonicalValue::Array(vec![text("a")])),
    ]);
    assert!(check_object_schema(&schema, "schema").is_ok());

    let open = object(vec![
        ("type", text("object")),
        ("properties", object(vec![("a", object(vec![]))])),
    ]);
    let error = check_object_schema(&open, "schema").expect_err("open schema");
    assert_eq!(error.message(), "schema must forbid additional properties");
}

#[test]
fn a_schema_may_not_declare_an_unknown_keyword() {
    let schema = object(vec![
        ("type", text("object")),
        ("additionalProperties", CanonicalValue::Bool(false)),
        ("properties", object(vec![("a", object(vec![]))])),
        ("nonsense", integer(1)),
    ]);
    let error = check_object_schema(&schema, "schema").expect_err("keyword");
    assert_eq!(error.message(), "unsupported schema keyword: nonsense");
    assert_eq!(error.path(), Some("schema"));
}

#[test]
fn a_value_is_validated_against_its_schema() {
    let schema = object(vec![
        ("type", text("object")),
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            object(vec![(
                "a",
                object(vec![("type", text("string")), ("maxLength", integer(4))]),
            )]),
        ),
        ("required", CanonicalValue::Array(vec![text("a")])),
    ]);
    assert!(validate_against_schema(&object(vec![("a", text("x"))]), &schema, "$", 0).is_ok());
    let error = validate_against_schema(&object(vec![("a", text("xxxxx"))]), &schema, "$", 0)
        .expect_err("length");
    assert_eq!(error.path(), Some("$.a"));
    assert_eq!(
        error.to_string(),
        "$.a: string is longer than the permitted length"
    );
    let error = validate_against_schema(
        &object(vec![("a", text("x")), ("z", text("y"))]),
        &schema,
        "$",
        0,
    )
    .expect_err("extra");
    assert_eq!(error.message(), "unpermitted properties: z");
    let error = validate_against_schema(&object(vec![("z", text("x"))]), &schema, "$", 0)
        .expect_err("missing");
    assert_eq!(error.message(), "required property 'a' is missing");
}

#[test]
fn a_schema_may_not_be_too_deep() {
    let mut schema = object(vec![("type", text("integer"))]);
    for _ in 0..40 {
        schema = object(vec![("allOf", CanonicalValue::Array(vec![schema]))]);
    }
    let error = validate_against_schema(&integer(1), &schema, "$", 0).expect_err("depth");
    assert_eq!(error.message(), "value does not match a required subschema");
}

// ---------------------------------------------------------------------------
// The environment of the test itself.
// ---------------------------------------------------------------------------

#[test]
fn the_portable_paths_of_the_crate_are_usable() {
    assert!(Path::new("/tmp").is_absolute());
}
