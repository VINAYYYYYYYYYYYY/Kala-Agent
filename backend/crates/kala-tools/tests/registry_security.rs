//! The security tests of the tool registry.
//!
//! The source is `tests/security/test_schema_redos.py` and
//! `tests/security/test_tool_executor.py`. The registry is the security
//! boundary of the tool plane: it decides what may be registered, which
//! capabilities a tool may claim and which registration may be trusted to
//! run inside the kernel process.

mod support;

use std::sync::Arc;

use kala_contracts::{ContractModel, SideEffectLevel, ToolDescriptor};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::protocol::{SubprocessToolHandler, canonical_json_digest};
use kala_tools::registry::{MAX_REGISTERED_TOOLS, ToolRegistry, ToolRegistryError};

use support::TestHandler;

/// The output schema of the security tests.
fn output_schema() -> CanonicalValue {
    CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "value",
                CanonicalValue::object([("type", CanonicalValue::text("string"))]),
            )]),
        ),
        (
            "required",
            CanonicalValue::array([CanonicalValue::text("value")]),
        ),
        ("type", CanonicalValue::text("object")),
    ])
}

/// A descriptor of the security tests.
fn descriptor(
    input_schema: &CanonicalValue,
    side_effect_level: SideEffectLevel,
    capabilities: &[&str],
    destructive: bool,
) -> ToolDescriptor {
    let mut entries = CanonicalObject::new();
    entries.insert(JsonText::new("tool_id"), CanonicalValue::text("test.tool"));
    entries.insert(JsonText::new("version"), CanonicalValue::text("1.0.0"));
    entries.insert(JsonText::new("name"), CanonicalValue::text("Test tool"));
    entries.insert(
        JsonText::new("description"),
        CanonicalValue::text("A test tool."),
    );
    entries.insert(
        JsonText::new("input_schema_digest"),
        CanonicalValue::text(canonical_json_digest(input_schema).expect("digest")),
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
    entries.insert(JsonText::new("timeout_seconds"), CanonicalValue::Float(5.0));
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

#[test]
fn the_registry_refuses_a_schema_whose_pattern_is_ambiguous() {
    let registry = ToolRegistry::new();
    let schema = CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "value",
                CanonicalValue::object([
                    ("maxLength", CanonicalValue::integer(32)),
                    ("pattern", CanonicalValue::text("^(a|aa)+$")),
                    ("type", CanonicalValue::text("string")),
                ]),
            )]),
        ),
        (
            "required",
            CanonicalValue::array([CanonicalValue::text("value")]),
        ),
        ("type", CanonicalValue::text("object")),
    ]);
    let descriptor = descriptor(&schema, SideEffectLevel::Read, &["design.read"], false);
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    let error = registry
        .register(descriptor, handler, schema, output_schema())
        .expect_err("refused");
    assert_eq!(error.python_class_name(), "ToolRegistrationError");
    assert!(error.to_string().contains("alternatives"));
    assert!(registry.tool_ids().expect("identifiers").is_empty());
}

#[test]
fn only_a_first_party_pure_evaluator_may_be_registered_as_trusted() {
    let registry = ToolRegistry::new();
    let schema = CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "value",
                CanonicalValue::object([("type", CanonicalValue::text("string"))]),
            )]),
        ),
        ("type", CanonicalValue::text("object")),
    ]);

    // A side effecting tool is refused.
    let impure = descriptor(
        &schema,
        SideEffectLevel::LocalWrite,
        &["artifact.write"],
        false,
    );
    let handler = Arc::new(TestHandler::first_party(impure.clone()));
    let error = registry
        .register_trusted_evaluator(impure, handler, schema.clone(), output_schema())
        .expect_err("refused");
    assert!(error.to_string().contains("non-destructive"));

    // A foreign handler is refused.
    let pure = descriptor(&schema, SideEffectLevel::None, &[], false);
    let handler = Arc::new(TestHandler::new(pure.clone()));
    let error = registry
        .register_trusted_evaluator(pure, handler, schema.clone(), output_schema())
        .expect_err("refused");
    assert!(error.to_string().contains("kernel package"));

    // A subprocess handler is refused.
    let pure = descriptor(&schema, SideEffectLevel::None, &[], false);
    let handler =
        SubprocessToolHandler::new(pure.clone(), "module:invoke", &["-B", "-s"]).expect("handler");
    let error = registry
        .register_trusted_evaluator(pure, Arc::new(handler), schema.clone(), output_schema())
        .expect_err("refused");
    assert!(error.to_string().contains("subprocess handler"));

    // Nothing was registered by the refused attempts.
    assert_eq!(registry.len(), 0);

    // A first party pure evaluator is accepted.
    let pure = descriptor(&schema, SideEffectLevel::None, &[], false);
    let handler = Arc::new(TestHandler::first_party(pure.clone()));
    let registered = registry
        .register_trusted_evaluator(pure.clone(), handler, schema, output_schema())
        .expect("registered");
    assert!(registered.trusted_in_process());
    assert_eq!(registered.side_effect_level(), SideEffectLevel::None);
    let found = registry.require("test.tool", Some("1.0.0")).expect("tool");
    assert!(found.trusted_in_process());
}

#[test]
fn a_privileged_capability_is_never_trusted() {
    let registry = ToolRegistry::new();
    let schema = CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "value",
                CanonicalValue::object([("type", CanonicalValue::text("string"))]),
            )]),
        ),
        ("type", CanonicalValue::text("object")),
    ]);
    for capability in [
        "egress",
        "export",
        "network",
        "network.access",
        "network.egress",
        "network.internet",
        "process.spawn",
        "shell.execute",
        "system.admin",
    ] {
        let descriptor = descriptor(&schema, SideEffectLevel::None, &[capability], false);
        let handler = Arc::new(TestHandler::first_party(descriptor.clone()));
        let error = registry
            .register_trusted_evaluator(descriptor, handler, schema.clone(), output_schema())
            .expect_err("refused");
        assert!(
            error.to_string().contains("non-destructive"),
            "the capability {capability} must not be trusted"
        );
    }
}

#[test]
fn a_side_effecting_tool_must_declare_its_capabilities() {
    let registry = ToolRegistry::new();
    let schema = CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "value",
                CanonicalValue::object([("type", CanonicalValue::text("string"))]),
            )]),
        ),
        ("type", CanonicalValue::text("object")),
    ]);
    for level in [
        SideEffectLevel::LocalWrite,
        SideEffectLevel::Network,
        SideEffectLevel::Egress,
    ] {
        let descriptor = descriptor(&schema, level, &[], false);
        let handler = Arc::new(TestHandler::new(descriptor.clone()));
        let error = registry
            .register(descriptor, handler, schema.clone(), output_schema())
            .expect_err("refused");
        assert!(
            error
                .to_string()
                .contains("declare its required capabilities")
        );
    }
    // A read only tool needs no capability.
    let descriptor = descriptor(&schema, SideEffectLevel::Read, &[], false);
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    registry
        .register(descriptor, handler, schema, output_schema())
        .expect("registered");
}

#[test]
fn a_duplicate_registration_is_refused() {
    let registry = ToolRegistry::new();
    let schema = CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "value",
                CanonicalValue::object([("type", CanonicalValue::text("string"))]),
            )]),
        ),
        ("type", CanonicalValue::text("object")),
    ]);
    let descriptor = descriptor(&schema, SideEffectLevel::Read, &["design.read"], false);
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    registry
        .register(
            descriptor.clone(),
            handler.clone(),
            schema.clone(),
            output_schema(),
        )
        .expect("registered");
    let error = registry
        .register(descriptor, handler, schema, output_schema())
        .expect_err("refused");
    assert_eq!(error.python_class_name(), "DuplicateToolRegistrationError");
    assert_eq!(registry.len(), 1);
}

#[test]
fn the_registry_capacity_is_bounded() {
    const { assert!(MAX_REGISTERED_TOOLS > 1) };
    let registry = ToolRegistry::with_limit(2);
    let schema = CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "value",
                CanonicalValue::object([("type", CanonicalValue::text("string"))]),
            )]),
        ),
        ("type", CanonicalValue::text("object")),
    ]);
    for version in ["1.0.0", "2.0.0", "3.0.0"] {
        let mut entries = CanonicalObject::new();
        entries.insert(JsonText::new("tool_id"), CanonicalValue::text("test.tool"));
        entries.insert(JsonText::new("version"), CanonicalValue::text(version));
        entries.insert(JsonText::new("name"), CanonicalValue::text("Test tool"));
        entries.insert(
            JsonText::new("description"),
            CanonicalValue::text("A test tool."),
        );
        entries.insert(
            JsonText::new("input_schema_digest"),
            CanonicalValue::text(canonical_json_digest(&schema).expect("digest")),
        );
        entries.insert(
            JsonText::new("output_schema_digest"),
            CanonicalValue::text(canonical_json_digest(&output_schema()).expect("digest")),
        );
        entries.insert(
            JsonText::new("side_effect_level"),
            CanonicalValue::text(SideEffectLevel::Read.as_str()),
        );
        entries.insert(
            JsonText::new("required_capabilities"),
            CanonicalValue::array([CanonicalValue::text("design.read")]),
        );
        entries.insert(JsonText::new("timeout_seconds"), CanonicalValue::Float(5.0));
        entries.insert(JsonText::new("destructive"), CanonicalValue::Bool(false));
        entries.insert(
            JsonText::new("supports_structured_output"),
            CanonicalValue::Bool(true),
        );
        entries.insert(JsonText::new("idempotent"), CanonicalValue::Bool(true));
        let descriptor =
            ToolDescriptor::from_value(&CanonicalValue::Object(entries)).expect("descriptor");
        let handler = Arc::new(TestHandler::new(descriptor.clone()));
        let outcome = registry.register(descriptor, handler, schema.clone(), output_schema());
        if version == "3.0.0" {
            let error = outcome.expect_err("refused");
            assert!(error.to_string().contains("registry is full"));
        } else {
            outcome.expect("registered");
        }
    }
    assert_eq!(registry.len(), 2);
}

#[test]
fn the_identifiers_and_the_versions_are_bounded() {
    let registry = ToolRegistry::new();
    for identifier in ["", "   ", &"x".repeat(129), "has space", "has\u{0}null"] {
        let error = registry.require(identifier, None).expect_err("refused");
        assert_eq!(error.python_class_name(), "ToolRegistryError");
    }
    for version in ["", &"x".repeat(65), "has space", "has\u{0}null"] {
        let error = registry
            .require("test.tool", Some(version))
            .expect_err("refused");
        assert_eq!(error.python_class_name(), "ToolRegistryError");
    }
}

#[test]
fn the_granted_capabilities_are_checked_exactly() {
    let registry = ToolRegistry::new();
    let schema = CanonicalValue::object([
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            CanonicalValue::object([(
                "value",
                CanonicalValue::object([("type", CanonicalValue::text("string"))]),
            )]),
        ),
        ("type", CanonicalValue::text("object")),
    ]);
    let descriptor = descriptor(&schema, SideEffectLevel::Read, &["design.read"], false);
    let handler = Arc::new(TestHandler::new(descriptor.clone()));
    registry
        .register(descriptor.clone(), handler, schema, output_schema())
        .expect("registered");
    let error = registry
        .require_capabilities("test.tool", Some("1.0.0"), &[])
        .expect_err("refused");
    assert_eq!(error.python_class_name(), "ToolCapabilityError");
    assert_eq!(
        error.missing_capabilities(),
        Some(["design.read".to_owned()].as_slice())
    );
    let found = registry
        .require_capabilities("test.tool", Some("1.0.0"), &["design.read".to_owned()])
        .expect("granted");
    assert_eq!(found.tool_id(), "test.tool");
}

#[test]
fn the_error_hierarchy_is_preserved() {
    let error = ToolRegistryError::registration("bad");
    assert_eq!(error.python_class_name(), "ToolRegistrationError");
    let error = ToolRegistryError::duplicate("bad");
    assert_eq!(error.python_class_name(), "DuplicateToolRegistrationError");
    assert!(error.is_registration());
    let error = ToolRegistryError::schema_digest_mismatch("bad");
    assert_eq!(error.python_class_name(), "SchemaDigestMismatchError");
    assert!(error.is_registration());
    let error = ToolRegistryError::unknown_tool("bad");
    assert_eq!(error.python_class_name(), "UnknownToolError");
    let error = ToolRegistryError::version_not_found("bad");
    assert_eq!(error.python_class_name(), "ToolVersionNotFoundError");
    assert!(error.is_unknown_tool());
    let error = ToolRegistryError::capability("bad", vec!["a".to_owned()]);
    assert_eq!(error.python_class_name(), "ToolCapabilityError");
    assert_eq!(
        error.missing_capabilities(),
        Some(["a".to_owned()].as_slice())
    );
}
