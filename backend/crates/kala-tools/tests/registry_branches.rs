//! The Rust port of the Python registry branch tests.
//!
//! The source is `tests/unit/test_registry_branches.py`. Every branch of the
//! registry that the Python implementation covers is exercised here, with the
//! same expectations and the same messages.

mod support;

use std::sync::Arc;

use kala_contracts::{ContractModel, SideEffectLevel, ToolDescriptor};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::protocol::{SubprocessToolHandler, canonical_json_digest};
use kala_tools::registry::{
    MAX_REGISTERED_TOOLS, RegisteredTool, ToolRegistry, ToolRegistryError, deep_freeze,
    frozen_schema, is_first_party, is_pure_evaluator, version_key,
};

use support::TestHandler;

/// The input schema of the test registrations.
fn input_schema() -> CanonicalValue {
    CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "operation",
                CanonicalValue::object([
                    (
                        "enum",
                        CanonicalValue::array([CanonicalValue::text("beam_volume")]),
                    ),
                    ("type", CanonicalValue::text("string")),
                ]),
            )]),
        ),
        (
            "required",
            CanonicalValue::array([CanonicalValue::text("operation")]),
        ),
        ("title", CanonicalValue::text("tool.input")),
        ("type", CanonicalValue::text("object")),
    ])
}

/// The output schema of the test registrations.
fn output_schema() -> CanonicalValue {
    CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "ok",
                CanonicalValue::object([("type", CanonicalValue::text("boolean"))]),
            )]),
        ),
        (
            "required",
            CanonicalValue::array([CanonicalValue::text("ok")]),
        ),
        ("title", CanonicalValue::text("tool.output")),
        ("type", CanonicalValue::text("object")),
    ])
}

/// A descriptor of the test registrations.
fn make_descriptor(
    tool_id: &str,
    version: &str,
    side_effect_level: SideEffectLevel,
    capabilities: &[&str],
    destructive: bool,
    timeout_seconds: f64,
) -> ToolDescriptor {
    let mut entries = CanonicalObject::new();
    entries.insert(JsonText::new("tool_id"), CanonicalValue::text(tool_id));
    entries.insert(JsonText::new("version"), CanonicalValue::text(version));
    entries.insert(JsonText::new("name"), CanonicalValue::text("Test tool"));
    entries.insert(
        JsonText::new("description"),
        CanonicalValue::text("A test tool."),
    );
    entries.insert(
        JsonText::new("input_schema_digest"),
        CanonicalValue::text(canonical_json_digest(&input_schema()).expect("digest")),
    );
    entries.insert(
        JsonText::new("output_schema_digest"),
        CanonicalValue::text(canonical_json_digest(&output_schema()).expect("digest")),
    );
    entries.insert(
        JsonText::new("side_effect_level"),
        CanonicalValue::text(side_effect_level.as_str()),
    );
    entries.insert(
        JsonText::new("required_capabilities"),
        CanonicalValue::array(
            capabilities
                .iter()
                .map(|capability| CanonicalValue::text(*capability)),
        ),
    );
    entries.insert(
        JsonText::new("timeout_seconds"),
        CanonicalValue::Float(timeout_seconds),
    );
    entries.insert(
        JsonText::new("destructive"),
        CanonicalValue::Bool(destructive),
    );
    entries.insert(
        JsonText::new("supports_structured_output"),
        CanonicalValue::Bool(true),
    );
    entries.insert(JsonText::new("idempotent"), CanonicalValue::Bool(true));
    ToolDescriptor::from_value(&CanonicalValue::Object(entries)).expect("descriptor")
}

/// The default descriptor of the test registrations.
fn default_descriptor() -> ToolDescriptor {
    make_descriptor(
        "mechanical.reference-evaluator",
        "1.0.0",
        SideEffectLevel::None,
        &[],
        false,
        10.0,
    )
}

/// A registered tool of the test registrations.
fn registered(tool_id: &str, version: &str, capabilities: &[&str]) -> RegisteredTool {
    let registry = ToolRegistry::new();
    let descriptor = make_descriptor(
        tool_id,
        version,
        SideEffectLevel::None,
        capabilities,
        false,
        10.0,
    );
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    registry
        .build(descriptor, handler, input_schema(), output_schema())
        .expect("registration")
}

#[test]
fn the_version_key_orders_numeric_segments_numerically() {
    assert!(version_key("1.0.0").expect("key") < version_key("1.0.10").expect("key"));
    assert!(version_key("2.0.0").expect("key") > version_key("1.99.99").expect("key"));
    assert!(version_key("1.0.0-alpha").expect("key") > version_key("1.0.0").expect("key"));
    assert!(version_key("1.0").expect("key") < version_key("1.0.0").expect("key"));
}

#[test]
fn the_first_party_detection_rejects_foreign_handlers() {
    let handler = TestHandler::new(default_descriptor());
    assert!(!is_first_party(&handler));
}

#[test]
fn the_first_party_detection_accepts_a_kernel_handler() {
    let handler = TestHandler::first_party(default_descriptor());
    assert!(is_first_party(&handler));
}

#[test]
fn a_pure_evaluator_requires_no_side_effects() {
    assert!(is_pure_evaluator(&default_descriptor()));
    assert!(!is_pure_evaluator(&make_descriptor(
        "mechanical.reference-evaluator",
        "1.0.0",
        SideEffectLevel::Network,
        &["artifact.write"],
        true,
        10.0,
    )));
    assert!(!is_pure_evaluator(&make_descriptor(
        "mechanical.reference-evaluator",
        "1.0.0",
        SideEffectLevel::LocalWrite,
        &["artifact.write"],
        false,
        10.0,
    )));
    assert!(is_pure_evaluator(&make_descriptor(
        "mechanical.reference-evaluator",
        "1.0.0",
        SideEffectLevel::None,
        &["artifact.read"],
        false,
        10.0,
    )));
}

#[test]
fn the_deep_freeze_returns_the_immutable_value() {
    let frozen = deep_freeze(&CanonicalValue::object([(
        "a",
        CanonicalValue::array([
            CanonicalValue::integer(1),
            CanonicalValue::object([("b", CanonicalValue::integer(2))]),
        ]),
    )]))
    .expect("frozen");
    assert_eq!(
        kala_tools::protocol::canonical_json_dumps(&frozen, kala_tools::protocol::MAX_JSON_BYTES)
            .expect("canonical"),
        "{\"a\":[1,{\"b\":2}]}"
    );
    let sequence = deep_freeze(&CanonicalValue::array([
        CanonicalValue::integer(1),
        CanonicalValue::integer(2),
    ]))
    .expect("frozen");
    assert!(frozen_eq(
        &sequence,
        &CanonicalValue::array([CanonicalValue::integer(1), CanonicalValue::integer(2)])
    ));
    let text = deep_freeze(&CanonicalValue::text("text")).expect("frozen");
    assert!(frozen_eq(&text, &CanonicalValue::text("text")));
}

#[test]
fn the_frozen_schema_freezes_the_top_level() {
    let frozen = frozen_schema(&input_schema()).expect("frozen");
    assert!(matches!(frozen, CanonicalValue::Object(_)));
}

#[test]
fn a_registered_tool_exposes_the_descriptor_fields() {
    let tool = registered(
        "mechanical.reference-evaluator",
        "1.0.0",
        &["artifact.read"],
    );
    assert_eq!(tool.tool_id(), "mechanical.reference-evaluator");
    assert_eq!(tool.version(), "1.0.0");
    assert_eq!(tool.required_capabilities().len(), 1);
    assert_eq!(tool.required_capabilities()[0].as_str(), "artifact.read");
    assert_eq!(tool.side_effect_level(), SideEffectLevel::None);
    assert_eq!(tool.timeout_seconds(), 10.0);
    assert!(!tool.trusted_in_process());
    let expected = canonical_json_digest(
        &tool
            .descriptor()
            .to_wire_value()
            .to_canonical_value()
            .expect("descriptor"),
    )
    .expect("digest");
    assert_eq!(tool.descriptor_digest(), expected);
}

#[test]
fn a_registered_tool_requires_a_matching_handler_descriptor() {
    let registry = ToolRegistry::new();
    let descriptor = default_descriptor();
    let other = make_descriptor(
        "mechanical.reference-evaluator",
        "9.9.9",
        SideEffectLevel::None,
        &[],
        false,
        10.0,
    );
    let handler = Arc::new(TestHandler::new(other));
    let error = registry
        .build(descriptor, handler, input_schema(), output_schema())
        .expect_err("mismatch");
    assert_eq!(error.python_class_name(), "ToolRegistrationError");
    assert!(
        error
            .to_string()
            .contains("handler descriptor does not match the registered descriptor")
    );
}

#[test]
fn a_registered_tool_enforces_the_schema_digests() {
    let _registry = ToolRegistry::new();
    let descriptor = default_descriptor();
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    let error = RegisteredTool::new(
        descriptor,
        handler,
        input_schema(),
        output_schema(),
        "0".repeat(64),
        canonical_json_digest(&output_schema()).expect("digest"),
    )
    .expect_err("mismatch");
    assert_eq!(error.python_class_name(), "SchemaDigestMismatchError");
    assert!(error.to_string().contains("input schema digest"));
}

#[test]
fn a_registered_tool_refuses_a_subprocess_trusted_handler() {
    let registry = ToolRegistry::new();
    let descriptor = default_descriptor();
    let handler = SubprocessToolHandler::new(descriptor.clone(), "module:invoke", &["-B", "-s"])
        .expect("handler");
    let error = registry
        .build_trusted_evaluator(
            descriptor,
            Arc::new(handler),
            input_schema(),
            output_schema(),
        )
        .expect_err("refused");
    assert_eq!(error.python_class_name(), "ToolRegistrationError");
    assert!(
        error
            .to_string()
            .contains("subprocess handler cannot run inside")
    );
}

#[test]
fn a_registered_tool_refuses_an_impure_trusted_evaluator() {
    let registry = ToolRegistry::new();
    let descriptor = make_descriptor(
        "mechanical.reference-evaluator",
        "1.0.0",
        SideEffectLevel::Network,
        &["artifact.write"],
        true,
        10.0,
    );
    let handler = Arc::new(TestHandler::first_party(descriptor.clone()));
    let error = registry
        .build_trusted_evaluator(descriptor, handler, input_schema(), output_schema())
        .expect_err("refused");
    assert!(error.to_string().contains("non-destructive tool"));
}

#[test]
fn a_registered_tool_refuses_a_foreign_trusted_evaluator() {
    let registry = ToolRegistry::new();
    let descriptor = default_descriptor();
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    let error = registry
        .build_trusted_evaluator(descriptor, handler, input_schema(), output_schema())
        .expect_err("refused");
    assert!(error.to_string().contains("kernel package"));
}

#[test]
fn a_registered_tool_validates_the_arguments_and_the_results() {
    let tool = registered("mechanical.reference-evaluator", "1.0.0", &[]);
    let arguments = CanonicalValue::object([("operation", CanonicalValue::text("beam_volume"))]);
    assert_eq!(
        tool.validate_arguments(&arguments).expect("arguments"),
        "{\"operation\":\"beam_volume\"}"
    );
    let result = CanonicalValue::object([("ok", CanonicalValue::Bool(true))]);
    assert_eq!(
        tool.validate_result(&result).expect("result"),
        "{\"ok\":true}"
    );
    assert!(
        tool.validate_arguments(&CanonicalValue::Object(CanonicalObject::new()))
            .is_err()
    );
    assert!(
        tool.validate_result(&CanonicalValue::Object(CanonicalObject::new()))
            .is_err()
    );
}

#[test]
fn a_registered_tool_reports_the_capability_authorization() {
    let tool = registered(
        "mechanical.reference-evaluator",
        "1.0.0",
        &["artifact.read", "tool.execute"],
    );
    assert!(tool.is_authorized_for(&["artifact.read".to_owned(), "tool.execute".to_owned()]));
    assert!(!tool.is_authorized_for(&["artifact.read".to_owned()]));
}

#[test]
fn the_verify_detects_a_mutated_handler_descriptor() {
    let registry = ToolRegistry::new();
    let descriptor = make_descriptor(
        "mechanical.reference-evaluator",
        "1.0.0",
        SideEffectLevel::None,
        &[],
        false,
        10.0,
    );
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    let tool = registry
        .build(descriptor, handler.clone(), input_schema(), output_schema())
        .expect("registration");
    handler.replace_descriptor(make_descriptor(
        "mechanical.reference-evaluator",
        "9.9.9",
        SideEffectLevel::None,
        &[],
        false,
        10.0,
    ));
    let error = tool.verify_immutable().expect_err("modified");
    assert!(error.to_string().contains("modified after registration"));
}

#[test]
fn the_verify_passes_for_an_untouched_tool() {
    let tool = registered("mechanical.reference-evaluator", "1.0.0", &[]);
    assert!(tool.verify_immutable().is_ok());
}

#[test]
fn the_registry_build_rejects_bad_descriptors() {
    let registry = ToolRegistry::new();
    let descriptor = make_descriptor(
        "mechanical.reference-simulator",
        "1.0.0",
        SideEffectLevel::LocalWrite,
        &[],
        false,
        10.0,
    );
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    let error = registry
        .build(descriptor, handler, input_schema(), output_schema())
        .expect_err("refused");
    assert!(
        error
            .to_string()
            .contains("declare its required capabilities")
    );
}

#[test]
fn the_registry_build_reports_malformed_schemas() {
    let registry = ToolRegistry::new();
    let descriptor = default_descriptor();
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    let error = registry
        .build(
            descriptor.clone(),
            handler.clone(),
            CanonicalValue::object([("type", CanonicalValue::text("string"))]),
            output_schema(),
        )
        .expect_err("refused");
    assert!(error.to_string().contains("input_schema"));
    let error = registry
        .build(
            descriptor,
            handler,
            input_schema(),
            CanonicalValue::object([("type", CanonicalValue::text("string"))]),
        )
        .expect_err("refused");
    assert!(error.to_string().contains("output_schema"));
}

#[test]
fn the_registry_build_trusted_evaluator_reports_the_registration_errors() {
    let registry = ToolRegistry::new();
    let _ = &registry;
    let descriptor = default_descriptor();
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    let error = registry
        .build_trusted_evaluator(
            descriptor,
            handler,
            CanonicalValue::object([("type", CanonicalValue::text("string"))]),
            output_schema(),
        )
        .expect_err("refused");
    assert_eq!(error.python_class_name(), "ToolRegistrationError");
}

#[test]
fn the_registry_accepts_prebuilt_tools() {
    let tool = registered("mechanical.reference-evaluator", "1.0.0", &[]);
    let registry = ToolRegistry::with_tools([tool.clone()]).expect("registry");
    assert_eq!(registry.len(), 1);
    let found = registry
        .require(tool.tool_id(), Some(tool.version()))
        .expect("tool");
    assert_eq!(found.tool_id(), tool.tool_id());
    assert_eq!(found.version(), tool.version());
}

#[test]
fn the_registry_refuses_duplicate_registrations() {
    let registry = ToolRegistry::new();
    let tool = registered("mechanical.reference-evaluator", "1.0.0", &[]);
    registry.register_tool(tool.clone()).expect("registered");
    let error = registry.register_tool(tool).expect_err("refused");
    assert_eq!(error.python_class_name(), "DuplicateToolRegistrationError");
    assert!(error.to_string().contains("already registered"));
}

#[test]
fn the_registry_enforces_the_registration_capacity() {
    const { assert!(MAX_REGISTERED_TOOLS > 1) };
    let registry = ToolRegistry::with_limit(1);
    let tool = registered("mechanical.reference-evaluator", "1.0.0", &[]);
    registry.register_tool(tool).expect("registered");
    let error = registry
        .register_tool(registered("mechanical.reference-evaluator", "2.0.0", &[]))
        .expect_err("refused");
    assert!(error.to_string().contains("registry is full"));
}

#[test]
fn the_registry_require_reports_unknown_tools() {
    let registry = ToolRegistry::new();
    let error = registry
        .require("mechanical.reference-evaluator", None)
        .expect_err("unknown");
    assert_eq!(error.python_class_name(), "UnknownToolError");
    assert!(error.to_string().contains("is not registered"));
}

#[test]
fn the_registry_require_reports_unknown_versions() {
    let registry = ToolRegistry::new();
    registry
        .register_tool(registered("mechanical.reference-evaluator", "1.0.0", &[]))
        .expect("registered");
    let error = registry
        .require("mechanical.reference-evaluator", Some("9.9.9"))
        .expect_err("unknown");
    assert_eq!(error.python_class_name(), "ToolVersionNotFoundError");
    assert!(error.to_string().contains("no registered version"));
}

#[test]
fn the_registry_require_without_a_version_returns_the_latest() {
    let registry = ToolRegistry::new();
    registry
        .register_tool(registered("mechanical.reference-evaluator", "1.9.0", &[]))
        .expect("registered");
    registry
        .register_tool(registered("mechanical.reference-evaluator", "1.10.0", &[]))
        .expect("registered");
    let latest = registry
        .require("mechanical.reference-evaluator", None)
        .expect("latest");
    assert_eq!(latest.version(), "1.10.0");
    let latest = registry
        .latest("mechanical.reference-evaluator")
        .expect("latest");
    assert_eq!(latest.version(), "1.10.0");
}

#[test]
fn the_registry_latest_reports_unknown_tools() {
    let error = ToolRegistry::new()
        .latest("mechanical.reference-evaluator")
        .expect_err("unknown");
    assert_eq!(error.python_class_name(), "UnknownToolError");
}

#[test]
fn the_registry_require_detects_a_mutated_handler_descriptor() {
    let registry = ToolRegistry::new();
    let descriptor = default_descriptor();
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    let tool = registry
        .build(descriptor, handler.clone(), input_schema(), output_schema())
        .expect("registration");
    registry.register_tool(tool).expect("registered");
    handler.replace_descriptor(make_descriptor(
        "mechanical.reference-evaluator",
        "9.9.9",
        SideEffectLevel::None,
        &[],
        false,
        10.0,
    ));
    let error = registry
        .require("mechanical.reference-evaluator", Some("1.0.0"))
        .expect_err("modified");
    assert!(error.to_string().contains("modified after registration"));
}

#[test]
fn the_registry_tools_requiring_filters_and_sorts() {
    let registry = ToolRegistry::new();
    registry
        .register_tool(registered(
            "mechanical.reference-evaluator",
            "1.0.0",
            &["artifact.read"],
        ))
        .expect("registered");
    registry
        .register_tool(registered(
            "mechanical.reference-evaluator",
            "1.10.0",
            &["artifact.read"],
        ))
        .expect("registered");
    registry
        .register_tool(registered("other.tool", "1.0.0", &["artifact.write"]))
        .expect("registered");
    let selected = registry.tools_requiring("artifact.read").expect("selected");
    let versions: Vec<&str> = selected.iter().map(|tool| tool.version()).collect();
    assert_eq!(versions, vec!["1.0.0", "1.10.0"]);
    assert!(
        registry
            .tools_requiring("tool.execute")
            .expect("selected")
            .is_empty()
    );
    let error = registry
        .tools_requiring("artifact read")
        .expect_err("refused");
    assert!(error.to_string().contains("contains unsafe characters"));
}

#[test]
fn the_registry_require_capabilities_reports_the_missing_grants() {
    let registry = ToolRegistry::new();
    let tool = registered(
        "mechanical.reference-evaluator",
        "1.0.0",
        &["artifact.read", "tool.execute"],
    );
    registry.register_tool(tool.clone()).expect("registered");
    let found = registry
        .require_capabilities(
            tool.tool_id(),
            None,
            &["artifact.read".to_owned(), "tool.execute".to_owned()],
        )
        .expect("granted");
    assert_eq!(found.tool_id(), tool.tool_id());
    let error = registry
        .require_capabilities(tool.tool_id(), None, &["artifact.read".to_owned()])
        .expect_err("missing");
    assert_eq!(error.python_class_name(), "ToolCapabilityError");
    assert_eq!(
        error.missing_capabilities(),
        Some(["tool.execute".to_owned()].as_slice())
    );
}

#[test]
fn the_registry_require_capabilities_validates_the_identifiers() {
    let registry = ToolRegistry::new();
    registry
        .register_tool(registered("mechanical.reference-evaluator", "1.0.0", &[]))
        .expect("registered");
    let error = registry
        .require_capabilities(
            "mechanical.reference-evaluator",
            None,
            &["granted capability".to_owned()],
        )
        .expect_err("refused");
    assert!(error.to_string().contains("granted capability"));
}

#[test]
fn the_registry_versions_lists_every_registered_version() {
    let registry = ToolRegistry::new();
    registry
        .register_tool(registered("mechanical.reference-evaluator", "1.9.0", &[]))
        .expect("registered");
    registry
        .register_tool(registered("mechanical.reference-evaluator", "1.10.0", &[]))
        .expect("registered");
    assert_eq!(
        registry
            .versions("mechanical.reference-evaluator")
            .expect("versions"),
        vec!["1.9.0".to_owned(), "1.10.0".to_owned()]
    );
    assert!(
        registry
            .versions("other.tool")
            .expect("versions")
            .is_empty()
    );
}

#[test]
fn the_registry_enumerates_the_tools_and_the_capabilities() {
    let registry = ToolRegistry::new();
    registry
        .register_tool(registered(
            "mechanical.reference-evaluator",
            "1.0.0",
            &["artifact.read"],
        ))
        .expect("registered");
    registry
        .register_tool(registered("other.tool", "1.0.0", &["artifact.write"]))
        .expect("registered");
    assert_eq!(
        registry.tool_ids().expect("identifiers"),
        vec![
            "mechanical.reference-evaluator".to_owned(),
            "other.tool".to_owned()
        ]
    );
    assert_eq!(
        registry.required_capabilities().expect("capabilities"),
        vec!["artifact.read".to_owned(), "artifact.write".to_owned()]
    );
    let tools = registry.all_tools().expect("tools");
    let identifiers: Vec<&str> = tools.iter().map(|tool| tool.tool_id()).collect();
    assert_eq!(
        identifiers,
        vec!["mechanical.reference-evaluator", "other.tool"]
    );
}

#[test]
fn the_registry_membership_accepts_only_identifier_pairs() {
    let registry = ToolRegistry::new();
    registry
        .register_tool(registered("mechanical.reference-evaluator", "1.0.0", &[]))
        .expect("registered");
    assert!(registry.contains("mechanical.reference-evaluator", "1.0.0"));
    assert!(!registry.contains("mechanical.reference-evaluator", "9.9.9"));
}

#[test]
fn the_registry_validates_the_identifiers_and_the_versions() {
    let registry = ToolRegistry::new();
    let error = registry.require("   ", None).expect_err("refused");
    assert!(error.to_string().contains("nonempty and bounded"));
    let error = registry
        .require(&"x".repeat(129), None)
        .expect_err("refused");
    assert!(error.to_string().contains("nonempty and bounded"));
    let error = registry.require("has space", None).expect_err("refused");
    assert!(error.to_string().contains("unsafe characters"));
    let error = registry.require("has\u{0}null", None).expect_err("refused");
    assert!(error.to_string().contains("unsafe characters"));
    let error = registry
        .require("mechanical.reference-evaluator", Some(&"x".repeat(65)))
        .expect_err("refused");
    assert!(error.to_string().contains("nonempty and bounded"));
    let error = registry
        .require("mechanical.reference-evaluator", Some("has space"))
        .expect_err("refused");
    assert!(error.to_string().contains("tool_version contains unsafe"));
}

#[test]
fn the_registry_trims_the_identifiers_before_the_lookup() {
    let registry = ToolRegistry::new();
    registry
        .register_tool(registered("mechanical.reference-evaluator", "1.0.0", &[]))
        .expect("registered");
    let found = registry
        .require("  mechanical.reference-evaluator  ", None)
        .expect("tool");
    assert_eq!(found.tool_id(), "mechanical.reference-evaluator");
    let found = registry
        .require("mechanical.reference-evaluator", Some(" 1.0.0 "))
        .expect("tool");
    assert_eq!(found.version(), "1.0.0");
}

#[test]
fn the_register_trusted_evaluator_registers_the_tool() {
    let registry = ToolRegistry::new();
    let descriptor = default_descriptor();
    let handler = Arc::new(TestHandler::first_party(descriptor.clone()));
    let tool = registry
        .register_trusted_evaluator(descriptor.clone(), handler, input_schema(), output_schema())
        .expect("registered");
    assert!(tool.trusted_in_process());
    let found = registry
        .require(
            descriptor.tool_id(),
            Some(descriptor.version().as_str().unwrap_or_default()),
        )
        .expect("tool");
    assert!(found.trusted_in_process());
}

#[test]
fn the_register_registers_a_built_tool() {
    let registry = ToolRegistry::new();
    let descriptor = default_descriptor();
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    let tool = registry
        .register(descriptor.clone(), handler, input_schema(), output_schema())
        .expect("registered");
    let found = registry
        .require(
            descriptor.tool_id(),
            Some(descriptor.version().as_str().unwrap_or_default()),
        )
        .expect("tool");
    assert_eq!(found.tool_id(), tool.tool_id());
    assert_eq!(found.version(), tool.version());
}

#[test]
fn the_registry_error_hierarchy_is_reported() {
    let error = ToolRegistryError::unknown_tool("tool x is not registered");
    assert_eq!(error.python_class_name(), "UnknownToolError");
    assert!(error.is_unknown_tool());
    let error = ToolRegistryError::registration("bad");
    assert!(error.is_registration());
    let error = ToolRegistryError::version_not_found("bad");
    assert!(error.is_unknown_tool());
}

/// Whether two values are equal.
fn frozen_eq(left: &CanonicalValue, right: &CanonicalValue) -> bool {
    left == right
}
