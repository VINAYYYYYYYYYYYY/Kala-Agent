//! The security and edge cases of the API schemas.
//!
//! The cases here are the ones a rejected or a hostile body must answer the
//! same way as the Python source answers them, and the ones that must not
//! exhaust the stack of the process.

use kala_api::schemas::{self, ApiModel};
use kala_api::validation::{Issue, Location, kind};
use kala_json::{CanonicalValue, Limits};

/// Parse a payload with the limits a request body is parsed with.
fn parse(payload: &str) -> Result<CanonicalValue, String> {
    kala_json::parse_strict(payload.as_bytes(), &schemas::request_body_limits())
        .map_err(|error| error.to_string())
}

/// The failures of one rejected request.
fn refusals(model: &str, payload: &str) -> Vec<Issue> {
    let value = parse(payload).expect("the payload is JSON");
    match model {
        "EvaluateDomainRequest" => schemas::EvaluateDomainRequest::from_value(&value)
            .map(|_| ())
            .expect_err("the payload is refused"),
        "CreateRevisionRequest" => schemas::CreateRevisionRequest::from_value(&value)
            .map(|_| ())
            .expect_err("the payload is refused"),
        "PrepareRevisionRequest" => schemas::PrepareRevisionRequest::from_value(&value)
            .map(|_| ())
            .expect_err("the payload is refused"),
        "RunMechanicalWorkflowRequest" => schemas::RunMechanicalWorkflowRequest::from_value(&value)
            .map(|_| ())
            .expect_err("the payload is refused"),
        "CheckpointDecisionRequest" => schemas::CheckpointDecisionRequest::from_value(&value)
            .map(|_| ())
            .expect_err("the payload is refused"),
        other => panic!("unknown model {other}"),
    }
}

#[test]
fn the_parameters_are_bounded_in_every_way() {
    let failures = refusals("EvaluateDomainRequest", r#"{"parameters":{"":"1"}}"#);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, kind::VALUE_ERROR);
    assert_eq!(
        failures[0].message,
        "Value error, parameter names must be nonempty and bounded"
    );

    let failures = refusals(
        "EvaluateDomainRequest",
        r#"{"parameters":{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":1}}"#,
    );
    assert_eq!(failures[0].kind, kind::VALUE_ERROR);

    let mut many = String::from("{\"parameters\":{");
    for index in 0..257 {
        if index > 0 {
            many.push(',');
        }
        many.push_str(&format!("\"k{index}\":{index}"));
    }
    many.push_str("}}");
    let failures = refusals("EvaluateDomainRequest", &many);
    assert_eq!(failures[0].kind, kind::VALUE_ERROR);
    assert_eq!(
        failures[0].message,
        "Value error, parameters cannot declare more than 256 entries"
    );
}

#[test]
fn the_parameters_are_bounded_in_size() {
    let payload = format!(
        "{{\"parameters\":{{\"a\":\"{}\"}}}}",
        "x".repeat(schemas::MAX_PARAMETERS_BYTES)
    );
    let failures = refusals("EvaluateDomainRequest", &payload);
    assert_eq!(failures[0].kind, kind::VALUE_ERROR);
    assert_eq!(
        failures[0].message,
        "Value error, canonical JSON exceeds the bounded size"
    );
}

#[test]
fn a_deep_parameters_value_is_refused_and_does_not_exhaust_the_stack() {
    let mut payload = String::from("{\"parameters\":");
    for _ in 0..200 {
        payload.push_str("{\"a\":");
    }
    payload.push('1');
    for _ in 0..200 {
        payload.push('}');
    }
    payload.push('}');
    let failures = refusals("EvaluateDomainRequest", &payload);
    assert_eq!(failures[0].kind, kind::VALUE_ERROR);
    assert_eq!(
        failures[0].message,
        "Value error, JSON nesting exceeds the supported depth"
    );
}

#[test]
fn a_very_deep_body_is_refused_by_the_parser_and_does_not_exhaust_the_stack() {
    let mut payload = String::new();
    for _ in 0..20_000 {
        payload.push('[');
    }
    for _ in 0..20_000 {
        payload.push(']');
    }
    // The parser refuses the document before the model reads it, and it does
    // so without recursing.
    assert!(parse(&payload).is_err());
}

#[test]
fn a_wide_parameters_value_is_refused_and_does_not_exhaust_the_stack() {
    let mut payload = String::from("{\"parameters\":{");
    for index in 0..2_000 {
        if index > 0 {
            payload.push(',');
        }
        payload.push_str(&format!("\"k{index}\":{index}"));
    }
    payload.push_str("}}");
    let failures = refusals("EvaluateDomainRequest", &payload);
    assert_eq!(failures[0].kind, kind::VALUE_ERROR);
    assert_eq!(
        failures[0].message,
        "Value error, parameters cannot declare more than 256 entries"
    );

    // A body that is larger than the size limit of the transport is refused
    // by the parser and not by the model.
    let mut payload = String::from("{\"parameters\":{");
    for index in 0..100_000 {
        if index > 0 {
            payload.push(',');
        }
        payload.push_str(&format!("\"k{index}\":{index}"));
    }
    payload.push_str("}}");
    assert!(parse(&payload).is_err());
}

#[test]
fn an_undeclared_field_is_refused() {
    let failures = refusals(
        "CreateRevisionRequest",
        r#"{"design_id":"a-design","domain":"building","role":"admin"}"#,
    );
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, kind::EXTRA_FORBIDDEN);
    assert_eq!(failures[0].message, "Extra inputs are not permitted");
    assert_eq!(failures[0].location_text(), vec!["role".to_owned()]);
}

#[test]
fn a_hostile_identifier_is_refused() {
    for payload in [
        r#"{"design_id":"a design","domain":"building"}"#,
        r#"{"design_id":"a-design;rm -rf /","domain":"building"}"#,
        r#"{"design_id":"../etc/passwd","domain":"building"}"#,
        r#"{"design_id":"a-design","domain":"building","parent_revision_id":"a/../b"}"#,
    ] {
        let failures = refusals("CreateRevisionRequest", payload);
        assert_eq!(failures[0].kind, kind::STRING_PATTERN_MISMATCH, "{payload}");
    }
}

#[test]
fn a_digest_that_is_not_a_digest_is_refused() {
    for payload in [
        r#"{"design_id":"a-design","domain":"building","state_digest":"A"}"#,
        r#"{"design_id":"a-design","domain":"building","state_digest":"../../etc/passwd"}"#,
        r#"{"design_id":"a-design","domain":"building","state_digest":"00000000000000000000000000000000000000000000000000000000000000000"}"#,
    ] {
        let failures = refusals("CreateRevisionRequest", payload);
        assert_eq!(failures[0].kind, kind::STRING_PATTERN_MISMATCH, "{payload}");
    }
}

#[test]
fn a_string_with_a_lone_surrogate_is_refused() {
    let value = kala_json::CanonicalValue::Text(
        kala_json::JsonText::from_code_points(vec![0xD800]).expect("a code point"),
    );
    let object = kala_json::CanonicalValue::Object(
        [(kala_json::JsonText::new("design_id"), value)]
            .into_iter()
            .collect(),
    );
    let failures = schemas::EvaluateDomainRequest::from_value(&object).expect_err("refused");
    assert_eq!(failures[0].kind, kind::STRING_UNICODE);
    assert_eq!(
        failures[0].message,
        "Input should be a valid string, unable to parse raw data as a unicode string"
    );
}

#[test]
fn the_request_models_strip_and_the_response_models_do_not() {
    let value = parse(r#"{"design_id":"  a-design  "}"#).expect("JSON");
    let request = schemas::EvaluateDomainRequest::from_value(&value).expect("accepted");
    assert_eq!(request.design_id.as_deref(), Some("a-design"));

    let value = parse(r#"{"status":"  ok  ","service":"s","version":"v","provider":"p","model":"m","offline":false,"egress_configured":false,"provider_endpoint_declared":false,"data_dir":"/tmp","schema_version":1,"domains":[],"disclaimer":""}"#)
        .expect("JSON");
    let response = schemas::HealthResponse::from_value(&value).expect("accepted");
    assert_eq!(response.status, "  ok  ");
}

#[test]
fn the_models_report_the_failures_in_the_order_they_declare_their_fields() {
    let failures = refusals(
        "CreateRevisionRequest",
        r#"{"domain":"nope","design_id":"a design","summary":5}"#,
    );
    let kinds: Vec<&str> = failures
        .iter()
        .map(|failure| failure.kind.as_str())
        .collect();
    assert_eq!(
        kinds,
        vec![kind::STRING_PATTERN_MISMATCH, kind::ENUM, kind::STRING_TYPE,]
    );
    let locations: Vec<Vec<String>> = failures
        .iter()
        .map(|failure| failure.location_text())
        .collect();
    assert_eq!(
        locations,
        vec![
            vec!["design_id".to_owned()],
            vec!["domain".to_owned()],
            vec!["summary".to_owned()],
        ]
    );
}

#[test]
fn a_rejected_body_never_carries_the_submitted_value_in_its_details() {
    let failures = refusals(
        "CreateRevisionRequest",
        r#"{"design_id":"a design","domain":"building","secret":"hunter2"}"#,
    );
    let transport = schemas::transport_failures(&failures);
    for failure in &transport {
        let message = failure.message.clone().unwrap_or_default();
        assert!(!message.contains("hunter2"), "{message}");
        assert!(!message.contains("a design"), "{message}");
    }
}

#[test]
fn the_location_of_a_failure_carries_the_body_prefix() {
    let failures = refusals(
        "CreateRevisionRequest",
        r#"{"design_id":"a design","domain":"building"}"#,
    );
    let transport = schemas::transport_failures(&failures);
    assert_eq!(
        transport[0].location,
        vec!["body".to_owned(), "design_id".to_owned()]
    );
    assert_eq!(
        transport[0].kind.as_deref(),
        Some(kind::STRING_PATTERN_MISMATCH)
    );
}

#[test]
fn the_canonical_json_of_a_model_is_the_sorted_document() {
    let value = parse(r#"{"design_id":"a-design"}"#).expect("JSON");
    let request = schemas::EvaluateDomainRequest::from_value(&value).expect("accepted");
    assert_eq!(
        request.to_canonical_json().expect("a canonical document"),
        r#"{"design_id":"a-design","parameters":null}"#
    );
    assert_eq!(
        request.to_wire_json(),
        r#"{"design_id":"a-design","parameters":null}"#
    );
}

#[test]
fn a_body_that_is_not_an_object_is_refused_by_the_body_layer() {
    let value = CanonicalValue::Integer(5.into());
    let failures = schemas::EvaluateDomainRequest::from_body(Some(&value)).expect_err("refused");
    assert_eq!(failures[0].kind, kind::MODEL_ATTRIBUTES_TYPE);
    assert_eq!(
        failures[0].message,
        "Input should be a valid dictionary or object to extract fields from"
    );
    let failures = schemas::EvaluateDomainRequest::from_body(None).expect_err("refused");
    assert_eq!(failures[0].kind, kind::MISSING);
    assert_eq!(failures[0].message, "Field required");
}

#[test]
fn a_huge_number_is_kept_as_it_is() {
    let value = parse(r#"{"parameters":{"a":123456789012345678901234567890}}"#).expect("JSON");
    let request = schemas::EvaluateDomainRequest::from_value(&value).expect("accepted");
    assert_eq!(
        request.to_canonical_json().expect("a canonical document"),
        r#"{"design_id":null,"parameters":{"a":123456789012345678901234567890}}"#
    );
}

#[test]
fn the_limits_of_a_body_are_the_limits_of_the_python_source() {
    let limits = schemas::request_body_limits();
    assert_eq!(limits.max_json_depth, 512);
    assert_eq!(
        limits.max_integer_digits,
        Limits::DEFAULT_MAX_INTEGER_DIGITS
    );
    // The bounded JSON domain of the tool protocol is applied by the model
    // that reads the body and not by the parser.
    assert_eq!(limits.max_string_length, usize::MAX);
    assert_eq!(limits.max_collection_items, usize::MAX);
    assert_eq!(limits.max_key_length, usize::MAX);
}

#[test]
fn the_models_are_values_and_not_shared_state() {
    let value = parse(r#"{"design_id":"a-design"}"#).expect("JSON");
    let first = schemas::EvaluateDomainRequest::from_value(&value).expect("accepted");
    let mut second = first.clone();
    second.design_id = Some("other-design".to_owned());
    assert_eq!(first.design_id.as_deref(), Some("a-design"));
    assert_eq!(second.design_id.as_deref(), Some("other-design"));
}

#[test]
fn a_failure_of_a_nested_model_is_reported_where_it_happened() {
    let value =
        parse(r#"{"count":1,"packages":[{"domain":"mechanical"}],"workflows":[],"domains":[]}"#)
            .expect("JSON");
    let failures = schemas::DomainListResponse::from_value(&value).expect_err("refused");
    let location: Vec<String> = failures[0].location_text();
    assert_eq!(
        location,
        vec![
            "packages".to_owned(),
            "0".to_owned(),
            "package_id".to_owned()
        ]
    );
    assert_eq!(failures[0].kind, kind::MISSING);
    assert_eq!(
        failures[0].location.last(),
        Some(&Location::Field("package_id".to_owned()))
    );
}
