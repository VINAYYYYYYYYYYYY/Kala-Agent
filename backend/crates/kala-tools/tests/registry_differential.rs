//! The live Python differential of the tool registry.
//!
//! The corpus is a scripted sequence of registry operations over a curated
//! and a randomised set of registrations, including hostile ones: a side
//! effecting tool without capabilities, a schema digest that does not match
//! the schema, a malformed schema, a duplicate registration, a foreign
//! handler that asks for the trusted in-process privilege and a handler that
//! changes its descriptor after the registration.
//!
//! Every operation is compared with the repository's own
//! `kala_agent.tools.registry`, byte for byte, including the class of the
//! exception the Python implementation raises.

mod support;

use std::sync::{Arc, Mutex};

use kala_contracts::{ContractModel, SideEffectLevel, ToolDescriptor};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::protocol::{
    ToolHandler, ToolProtocolError, ToolRequestEnvelope, ToolResponseEnvelope,
    canonical_json_digest,
};
use kala_tools::registry::{
    KERNEL_PACKAGE_DIRECTORY, RegisteredTool, RegistryHandler, ToolRegistry, ToolRegistryError,
};

use support::{Lcg, canonical, run_oracle};

// ---------------------------------------------------------------------------
// The test handler
// ---------------------------------------------------------------------------

/// A handler that can pretend to be first party and that can change its
/// descriptor after the registration.
struct TestHandler {
    descriptor: ToolDescriptor,
    replacement: Mutex<Option<ToolDescriptor>>,
    first_party: bool,
}

impl TestHandler {
    fn new(descriptor: ToolDescriptor, first_party: bool) -> Self {
        Self {
            descriptor,
            replacement: Mutex::new(None),
            first_party,
        }
    }

    fn replace_descriptor(&self, descriptor: ToolDescriptor) {
        let mut replacement = self.replacement.lock().expect("handler");
        *replacement = Some(descriptor);
    }
}

impl ToolHandler for TestHandler {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn invoke(
        &self,
        _request: &ToolRequestEnvelope,
    ) -> Result<ToolResponseEnvelope, ToolProtocolError> {
        Err(ToolProtocolError::protocol("the test handler does not run"))
    }
}

impl RegistryHandler for TestHandler {
    fn descriptor_snapshot(&self) -> ToolDescriptor {
        let replacement = self.replacement.lock().expect("handler");
        match replacement.as_ref() {
            None => self.descriptor.clone(),
            Some(descriptor) => descriptor.clone(),
        }
    }

    fn defining_package(&self) -> &'static str {
        if self.first_party {
            KERNEL_PACKAGE_DIRECTORY
        } else {
            ""
        }
    }
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

/// The input schema of the curated registrations.
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

/// The output schema of the curated registrations.
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

/// A registration case.
struct Case {
    tool_id: String,
    version: String,
    side_effect_level: String,
    required_capabilities: Vec<String>,
    destructive: bool,
    timeout_seconds: f64,
    input_schema: CanonicalValue,
    output_schema: CanonicalValue,
    input_schema_digest: Option<String>,
    output_schema_digest: Option<String>,
    first_party: bool,
    operations: Vec<CanonicalValue>,
}

impl Case {
    fn new(tool_id: &str, version: &str) -> Self {
        Self {
            tool_id: tool_id.to_owned(),
            version: version.to_owned(),
            side_effect_level: "none".to_owned(),
            required_capabilities: Vec::new(),
            destructive: false,
            timeout_seconds: 10.0,
            input_schema: input_schema(),
            output_schema: output_schema(),
            input_schema_digest: None,
            output_schema_digest: None,
            first_party: false,
            operations: Vec::new(),
        }
    }

    fn side_effect(mut self, level: &str) -> Self {
        self.side_effect_level = level.to_owned();
        self
    }

    fn capabilities(mut self, capabilities: &[&str]) -> Self {
        self.required_capabilities = capabilities
            .iter()
            .map(|value| (*value).to_owned())
            .collect();
        self
    }

    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    fn timeout(mut self, seconds: f64) -> Self {
        self.timeout_seconds = seconds;
        self
    }

    fn input_schema(mut self, schema: CanonicalValue) -> Self {
        self.input_schema = schema;
        self
    }

    fn output_schema(mut self, schema: CanonicalValue) -> Self {
        self.output_schema = schema;
        self
    }

    fn input_digest(mut self, digest: &str) -> Self {
        self.input_schema_digest = Some(digest.to_owned());
        self
    }

    fn output_digest(mut self, digest: &str) -> Self {
        self.output_schema_digest = Some(digest.to_owned());
        self
    }

    fn first_party(mut self) -> Self {
        self.first_party = true;
        self
    }

    fn with(mut self, operations: Vec<CanonicalValue>) -> Self {
        self.operations = operations;
        self
    }

    /// The corpus document of the case.
    fn to_value(&self) -> CanonicalValue {
        let mut descriptor = CanonicalObject::new();
        descriptor.insert(
            JsonText::new("tool_id"),
            CanonicalValue::text(&self.tool_id),
        );
        descriptor.insert(
            JsonText::new("version"),
            CanonicalValue::text(&self.version),
        );
        descriptor.insert(
            JsonText::new("side_effect_level"),
            CanonicalValue::text(&self.side_effect_level),
        );
        descriptor.insert(
            JsonText::new("required_capabilities"),
            CanonicalValue::array(self.required_capabilities.iter().map(CanonicalValue::text)),
        );
        descriptor.insert(
            JsonText::new("destructive"),
            CanonicalValue::Bool(self.destructive),
        );
        descriptor.insert(
            JsonText::new("timeout_seconds"),
            CanonicalValue::Float(self.timeout_seconds),
        );
        if let Some(digest) = &self.input_schema_digest {
            descriptor.insert(
                JsonText::new("input_schema_digest"),
                CanonicalValue::text(digest),
            );
        }
        if let Some(digest) = &self.output_schema_digest {
            descriptor.insert(
                JsonText::new("output_schema_digest"),
                CanonicalValue::text(digest),
            );
        }
        CanonicalValue::object([
            ("descriptor", CanonicalValue::Object(descriptor)),
            (
                "handler",
                CanonicalValue::object([("first_party", CanonicalValue::Bool(self.first_party))]),
            ),
            ("input_schema", self.input_schema.clone()),
            ("output_schema", self.output_schema.clone()),
            ("operations", CanonicalValue::array(self.operations.clone())),
        ])
    }
}

/// One operation of a case.
fn op(name: &str) -> CanonicalValue {
    CanonicalValue::object([("op", CanonicalValue::text(name))])
}

/// One operation of a case with members.
fn op_with(name: &str, members: &[(&str, CanonicalValue)]) -> CanonicalValue {
    let mut operation = CanonicalObject::new();
    operation.insert(JsonText::new("op"), CanonicalValue::text(name));
    for (key, value) in members {
        operation.insert(JsonText::new(key), value.clone());
    }
    CanonicalValue::Object(operation)
}

/// The curated corpus.
fn curated() -> Vec<Case> {
    let mut cases: Vec<Case> = Vec::new();

    // A pure evaluator, registered and looked up.
    cases.extend([
        Case::new("mechanical.reference-evaluator", "1.0.0").with(vec![
            op("build"),
            op("register"),
            op_with(
                "require",
                &[
                    (
                        "tool_id",
                        CanonicalValue::text("mechanical.reference-evaluator"),
                    ),
                    ("tool_version", CanonicalValue::text("1.0.0")),
                ],
            ),
            op_with(
                "require",
                &[(
                    "tool_id",
                    CanonicalValue::text("mechanical.reference-evaluator"),
                )],
            ),
            op_with(
                "latest",
                &[(
                    "tool_id",
                    CanonicalValue::text("mechanical.reference-evaluator"),
                )],
            ),
            op_with(
                "versions",
                &[(
                    "tool_id",
                    CanonicalValue::text("mechanical.reference-evaluator"),
                )],
            ),
            op("all_tools"),
            op("tool_ids"),
            op("required_capabilities"),
            op("len"),
            op_with(
                "contains",
                &[
                    (
                        "tool_id",
                        CanonicalValue::text("mechanical.reference-evaluator"),
                    ),
                    ("tool_version", CanonicalValue::text("1.0.0")),
                ],
            ),
            op_with(
                "contains",
                &[
                    (
                        "tool_id",
                        CanonicalValue::text("mechanical.reference-evaluator"),
                    ),
                    ("tool_version", CanonicalValue::text("9.9.9")),
                ],
            ),
        ]),
    ]);

    // A first party pure evaluator may be trusted.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .first_party()
            .with(vec![
                op("build_trusted"),
                op("register_trusted"),
                op_with(
                    "require",
                    &[(
                        "tool_id",
                        CanonicalValue::text("mechanical.reference-evaluator"),
                    )],
                ),
                op_with("verify_immutable", &[("index", CanonicalValue::integer(0))]),
            ]),
    );

    // A foreign handler may not be trusted.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .with(vec![op("build_trusted"), op("register_trusted")]),
    );

    // A privileged or side effecting tool may not be trusted.
    cases.push(
        Case::new("mechanical.reference-simulator", "1.0.0")
            .first_party()
            .side_effect("local_write")
            .capabilities(&["artifact.write"])
            .with(vec![op("build_trusted")]),
    );
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .first_party()
            .capabilities(&["network.access"])
            .with(vec![op("build_trusted")]),
    );
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .first_party()
            .capabilities(&["artifact.read"])
            .with(vec![op("build_trusted")]),
    );

    // A side effecting tool must declare its capabilities.
    cases.push(
        Case::new("mechanical.reference-simulator", "1.0.0")
            .side_effect("local_write")
            .with(vec![op("build"), op("register")]),
    );
    cases.push(
        Case::new("mechanical.reference-simulator", "1.0.0")
            .side_effect("network")
            .with(vec![op("build")]),
    );
    cases.push(
        Case::new("mechanical.reference-simulator", "1.0.0")
            .side_effect("egress")
            .with(vec![op("build")]),
    );

    // The schema digests must match the schemas.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .input_digest(&"0".repeat(64))
            .with(vec![op("build")]),
    );
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .output_digest(&"0".repeat(64))
            .with(vec![op("build")]),
    );

    // The schemas must be admitted object schemas.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .input_schema(CanonicalValue::object([(
                "type",
                CanonicalValue::text("string"),
            )]))
            .with(vec![op("build")]),
    );
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .output_schema(CanonicalValue::object([(
                "type",
                CanonicalValue::text("string"),
            )]))
            .with(vec![op("build")]),
    );
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .input_schema(CanonicalValue::array([CanonicalValue::text("nope")]))
            .with(vec![op("build")]),
    );

    // A duplicate registration is refused.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0").with(vec![
            op("build"),
            op("register"),
            op("register"),
            op_with("register_built", &[("index", CanonicalValue::integer(0))]),
            op("len"),
        ]),
    );

    // An unknown tool and an unknown version are refused differently.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0").with(vec![
            op("register"),
            op_with(
                "require",
                &[("tool_id", CanonicalValue::text("other.tool"))],
            ),
            op_with(
                "require",
                &[
                    (
                        "tool_id",
                        CanonicalValue::text("mechanical.reference-evaluator"),
                    ),
                    ("tool_version", CanonicalValue::text("9.9.9")),
                ],
            ),
            op_with("latest", &[("tool_id", CanonicalValue::text("other.tool"))]),
            op_with(
                "versions",
                &[("tool_id", CanonicalValue::text("other.tool"))],
            ),
        ]),
    );

    // The identifiers and the versions are validated.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0").with(vec![
            op_with("require", &[("tool_id", CanonicalValue::text("   "))]),
            op_with(
                "require",
                &[("tool_id", CanonicalValue::text("x".repeat(129)))],
            ),
            op_with("require", &[("tool_id", CanonicalValue::text("has space"))]),
            op_with(
                "require",
                &[("tool_id", CanonicalValue::text("has\u{0}null"))],
            ),
            op_with(
                "require",
                &[
                    (
                        "tool_id",
                        CanonicalValue::text("mechanical.reference-evaluator"),
                    ),
                    ("tool_version", CanonicalValue::text("x".repeat(65))),
                ],
            ),
            op_with(
                "require",
                &[
                    (
                        "tool_id",
                        CanonicalValue::text("mechanical.reference-evaluator"),
                    ),
                    ("tool_version", CanonicalValue::text("has space")),
                ],
            ),
            op_with(
                "require",
                &[
                    (
                        "tool_id",
                        CanonicalValue::text("mechanical.reference-evaluator"),
                    ),
                    ("tool_version", CanonicalValue::text("")),
                ],
            ),
        ]),
    );

    // The identifiers are trimmed before the lookup.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0").with(vec![
            op("register"),
            op_with(
                "require",
                &[(
                    "tool_id",
                    CanonicalValue::text("  mechanical.reference-evaluator  "),
                )],
            ),
            op_with(
                "require",
                &[
                    (
                        "tool_id",
                        CanonicalValue::text("mechanical.reference-evaluator"),
                    ),
                    ("tool_version", CanonicalValue::text(" 1.0.0 ")),
                ],
            ),
        ]),
    );

    // The capabilities are checked before the call is dispatched.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .capabilities(&["artifact.read", "tool.execute"])
            .with(vec![
                op("register"),
                op_with(
                    "require_capabilities",
                    &[
                        (
                            "tool_id",
                            CanonicalValue::text("mechanical.reference-evaluator"),
                        ),
                        (
                            "granted",
                            CanonicalValue::array([
                                CanonicalValue::text("artifact.read"),
                                CanonicalValue::text("tool.execute"),
                            ]),
                        ),
                    ],
                ),
                op_with(
                    "require_capabilities",
                    &[
                        (
                            "tool_id",
                            CanonicalValue::text("mechanical.reference-evaluator"),
                        ),
                        (
                            "granted",
                            CanonicalValue::array([CanonicalValue::text("artifact.read")]),
                        ),
                    ],
                ),
                op_with(
                    "require_capabilities",
                    &[
                        (
                            "tool_id",
                            CanonicalValue::text("mechanical.reference-evaluator"),
                        ),
                        (
                            "granted",
                            CanonicalValue::array([CanonicalValue::text("granted capability")]),
                        ),
                    ],
                ),
                op_with(
                    "require_capabilities",
                    &[
                        (
                            "tool_id",
                            CanonicalValue::text("mechanical.reference-evaluator"),
                        ),
                        ("granted", CanonicalValue::array([CanonicalValue::text("")])),
                    ],
                ),
            ]),
    );

    // The tools that require a capability are filtered and sorted.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .capabilities(&["artifact.read"])
            .with(vec![
                op("register"),
                op_with(
                    "tools_requiring",
                    &[("capability", CanonicalValue::text("artifact.read"))],
                ),
                op_with(
                    "tools_requiring",
                    &[("capability", CanonicalValue::text("tool.execute"))],
                ),
                op_with(
                    "tools_requiring",
                    &[("capability", CanonicalValue::text("artifact read"))],
                ),
                op_with(
                    "tools_requiring",
                    &[("capability", CanonicalValue::text(""))],
                ),
            ]),
    );

    // The versions are ordered numerically.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.9.0").with(vec![
            op("register"),
            op_with(
                "require",
                &[(
                    "tool_id",
                    CanonicalValue::text("mechanical.reference-evaluator"),
                )],
            ),
        ]),
    );
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.10.0").with(vec![
            op("register"),
            op_with(
                "require",
                &[(
                    "tool_id",
                    CanonicalValue::text("mechanical.reference-evaluator"),
                )],
            ),
            op_with(
                "versions",
                &[(
                    "tool_id",
                    CanonicalValue::text("mechanical.reference-evaluator"),
                )],
            ),
        ]),
    );
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0-alpha").with(vec![
            op("register"),
            op_with(
                "require",
                &[(
                    "tool_id",
                    CanonicalValue::text("mechanical.reference-evaluator"),
                )],
            ),
        ]),
    );

    // A handler that changes its descriptor after the registration is
    // detected.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0").with(vec![
            op("build"),
            op("register"),
            op("tamper_handler"),
            op_with("verify_immutable", &[("index", CanonicalValue::integer(0))]),
            op_with(
                "require",
                &[(
                    "tool_id",
                    CanonicalValue::text("mechanical.reference-evaluator"),
                )],
            ),
        ]),
    );

    // A destructive tool must declare a side effect.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .side_effect("read")
            .destructive()
            .with(vec![op("build")]),
    );

    // The timeout is bounded.
    cases.push(
        Case::new("mechanical.reference-evaluator", "1.0.0")
            .timeout(0.0)
            .with(vec![op("build")]),
    );

    cases
}

/// The randomised corpus.
fn randomised(seed: u64, count: usize) -> Vec<Case> {
    let mut generator = Lcg::new(seed);
    let side_effects = ["none", "read", "local_write", "network", "egress"];
    let capabilities = [
        "artifact.read",
        "artifact.write",
        "design.read",
        "tool.execute",
        "network.access",
    ];
    let identifiers = [
        "a.tool",
        "b.tool",
        "mechanical.reference-evaluator",
        "other.tool",
    ];
    let versions = ["1.0.0", "1.10.0", "1.9.0", "2.0.0", "0.1", "10.0.0"];
    let mut cases: Vec<Case> = Vec::new();
    for index in 0..count {
        let tool_id = identifiers[generator.below(identifiers.len() as u64) as usize];
        let version = versions[generator.below(versions.len() as u64) as usize];
        let side_effect = side_effects[generator.below(side_effects.len() as u64) as usize];
        let capability_count = generator.below(3) as usize;
        let mut chosen: Vec<&str> = Vec::new();
        for _ in 0..capability_count {
            let capability = capabilities[generator.below(capabilities.len() as u64) as usize];
            if !chosen.contains(&capability) {
                chosen.push(capability);
            }
        }
        let destructive = side_effect != "none" && generator.below(4) == 0;
        let first_party = generator.below(2) == 0;
        let mut case = Case::new(tool_id, version)
            .side_effect(side_effect)
            .capabilities(&chosen)
            .with(Vec::new());
        if destructive {
            case = case.destructive();
        }
        if first_party {
            case = case.first_party();
        }
        if generator.below(8) == 0 {
            case = case.input_digest(&"0".repeat(64));
        }
        if generator.below(8) == 0 {
            case = case.output_schema(CanonicalValue::object([(
                "type",
                CanonicalValue::text("string"),
            )]));
        }
        let operation_count = 3 + generator.below(5) as usize;
        let mut operations: Vec<CanonicalValue> = Vec::new();
        for _ in 0..operation_count {
            let choice = generator.below(10);
            operations.push(match choice {
                0 => op("build"),
                1 => op("build_trusted"),
                2 => op("register"),
                3 => op("register_trusted"),
                4 => op_with(
                    "require",
                    &[
                        ("tool_id", CanonicalValue::text(tool_id)),
                        ("tool_version", CanonicalValue::text(version)),
                    ],
                ),
                5 => op_with("require", &[("tool_id", CanonicalValue::text(tool_id))]),
                6 => op_with(
                    "tools_requiring",
                    &[("capability", CanonicalValue::text("artifact.read"))],
                ),
                7 => op_with("versions", &[("tool_id", CanonicalValue::text(tool_id))]),
                8 => op("all_tools"),
                _ => op_with(
                    "require_capabilities",
                    &[
                        ("tool_id", CanonicalValue::text(tool_id)),
                        (
                            "granted",
                            CanonicalValue::array([CanonicalValue::text("artifact.read")]),
                        ),
                    ],
                ),
            });
        }
        let _ = index;
        cases.push(case.with(operations));
    }
    cases
}

// ---------------------------------------------------------------------------
// The native interpreter
// ---------------------------------------------------------------------------

/// The summary of a registration, as the oracle reports it.
fn tool_summary(tool: &RegisteredTool) -> CanonicalValue {
    CanonicalValue::object([
        (
            "descriptor_digest",
            CanonicalValue::text(tool.descriptor_digest()),
        ),
        (
            "input_schema_digest",
            CanonicalValue::text(tool.input_schema_digest()),
        ),
        (
            "output_schema_digest",
            CanonicalValue::text(tool.output_schema_digest()),
        ),
        (
            "required_capabilities",
            CanonicalValue::array(
                tool.required_capabilities()
                    .iter()
                    .map(|capability| CanonicalValue::text(capability.as_str())),
            ),
        ),
        (
            "side_effect_level",
            CanonicalValue::text(tool.side_effect_level().as_str()),
        ),
        (
            "timeout_seconds",
            CanonicalValue::Float(tool.timeout_seconds()),
        ),
        ("tool_id", CanonicalValue::text(tool.tool_id())),
        (
            "trusted_in_process",
            CanonicalValue::Bool(tool.trusted_in_process()),
        ),
        ("version", CanonicalValue::text(tool.version())),
    ])
}

/// The native failure of one operation.
enum NativeFailure {
    /// A failure raised by the registry.
    Registry(ToolRegistryError),
    /// The `IndexError` CPython raises for an index that is out of range.
    Index,
}

impl NativeFailure {
    fn class_name(&self) -> &'static str {
        match self {
            Self::Registry(error) => error.python_class_name(),
            Self::Index => "IndexError",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Registry(error) => error.to_string(),
            Self::Index => "list index out of range".to_owned(),
        }
    }
}

/// The native result of one operation.
fn native_operation_full(
    registry: &ToolRegistry,
    descriptor: &ToolDescriptor,
    handler: &Arc<TestHandler>,
    input_schema: &CanonicalValue,
    output_schema: &CanonicalValue,
    built: &mut Vec<RegisteredTool>,
    operation: &CanonicalValue,
) -> Result<CanonicalValue, NativeFailure> {
    let CanonicalValue::Object(operation) = operation else {
        panic!("operation is an object");
    };
    let name = operation
        .get(&JsonText::new("op"))
        .and_then(|value| value.as_text())
        .map(|text| text.to_string_lossy())
        .unwrap_or_default();
    let text = |key: &str| {
        operation
            .get(&JsonText::new(key))
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy())
    };
    let granted = |key: &str| -> Vec<String> {
        match operation.get(&JsonText::new(key)) {
            Some(CanonicalValue::Array(items)) => items
                .iter()
                .filter_map(|item| item.as_text().map(|text| text.to_string_lossy()))
                .collect(),
            _ => Vec::new(),
        }
    };
    let index = |key: &str| -> usize {
        match operation.get(&JsonText::new(key)) {
            Some(CanonicalValue::Integer(number)) => number.to_string().parse().unwrap_or(0),
            _ => 0,
        }
    };
    match name.as_str() {
        "build" => {
            let tool = registry
                .build(
                    descriptor.clone(),
                    handler.clone(),
                    input_schema.clone(),
                    output_schema.clone(),
                )
                .map_err(NativeFailure::Registry)?;
            built.push(tool.clone());
            Ok(tool_summary(&tool))
        }
        "build_trusted" => {
            let tool = registry
                .build_trusted_evaluator(
                    descriptor.clone(),
                    handler.clone(),
                    input_schema.clone(),
                    output_schema.clone(),
                )
                .map_err(NativeFailure::Registry)?;
            built.push(tool.clone());
            Ok(tool_summary(&tool))
        }
        "register" => {
            let tool = registry
                .register(
                    descriptor.clone(),
                    handler.clone(),
                    input_schema.clone(),
                    output_schema.clone(),
                )
                .map_err(NativeFailure::Registry)?;
            built.push(tool.clone());
            Ok(tool_summary(&tool))
        }
        "register_trusted" => {
            let tool = registry
                .register_trusted_evaluator(
                    descriptor.clone(),
                    handler.clone(),
                    input_schema.clone(),
                    output_schema.clone(),
                )
                .map_err(NativeFailure::Registry)?;
            built.push(tool.clone());
            Ok(tool_summary(&tool))
        }
        "register_built" => {
            let Some(source) = built.get(index("index")) else {
                return Err(NativeFailure::Index);
            };
            let tool = registry
                .register_tool(source.clone())
                .map_err(NativeFailure::Registry)?;
            built.push(tool.clone());
            Ok(tool_summary(&tool))
        }
        "require" => {
            let tool = registry
                .require(
                    &text("tool_id").unwrap_or_default(),
                    text("tool_version").as_deref(),
                )
                .map_err(NativeFailure::Registry)?;
            Ok(tool_summary(&tool))
        }
        "latest" => {
            let tool = registry
                .latest(&text("tool_id").unwrap_or_default())
                .map_err(NativeFailure::Registry)?;
            Ok(tool_summary(&tool))
        }
        "tools_requiring" => {
            let tools = registry
                .tools_requiring(&text("capability").unwrap_or_default())
                .map_err(NativeFailure::Registry)?;
            Ok(CanonicalValue::array(
                tools.iter().map(tool_summary).collect::<Vec<_>>(),
            ))
        }
        "require_capabilities" => {
            let tool = registry
                .require_capabilities(
                    &text("tool_id").unwrap_or_default(),
                    text("tool_version").as_deref(),
                    &granted("granted"),
                )
                .map_err(NativeFailure::Registry)?;
            Ok(tool_summary(&tool))
        }
        "versions" => {
            let versions = registry
                .versions(&text("tool_id").unwrap_or_default())
                .map_err(NativeFailure::Registry)?;
            Ok(CanonicalValue::array(
                versions
                    .iter()
                    .map(CanonicalValue::text)
                    .collect::<Vec<_>>(),
            ))
        }
        "all_tools" => {
            let tools = registry.all_tools().map_err(NativeFailure::Registry)?;
            Ok(CanonicalValue::array(
                tools.iter().map(tool_summary).collect::<Vec<_>>(),
            ))
        }
        "tool_ids" => {
            let identifiers = registry.tool_ids().map_err(NativeFailure::Registry)?;
            Ok(CanonicalValue::array(
                identifiers
                    .iter()
                    .map(CanonicalValue::text)
                    .collect::<Vec<_>>(),
            ))
        }
        "required_capabilities" => {
            let capabilities = registry
                .required_capabilities()
                .map_err(NativeFailure::Registry)?;
            Ok(CanonicalValue::array(
                capabilities
                    .iter()
                    .map(CanonicalValue::text)
                    .collect::<Vec<_>>(),
            ))
        }
        "len" => Ok(CanonicalValue::integer(registry.len() as i64)),
        "contains" => Ok(CanonicalValue::Bool(registry.contains(
            &text("tool_id").unwrap_or_default(),
            &text("tool_version").unwrap_or_default(),
        ))),
        "tamper_handler" => {
            let mut replacement = descriptor.clone();
            replacement = descriptor_with_version(&replacement, "9.9.9");
            handler.replace_descriptor(replacement);
            Ok(CanonicalValue::Null)
        }
        "verify_immutable" => {
            let Some(tool) = built.get(index("index")) else {
                return Err(NativeFailure::Index);
            };
            tool.verify_immutable().map_err(NativeFailure::Registry)?;
            Ok(tool_summary(tool))
        }
        other => panic!("unknown operation: {other}"),
    }
}

/// The descriptor with another version.
fn descriptor_with_version(descriptor: &ToolDescriptor, version: &str) -> ToolDescriptor {
    let mut entries = CanonicalObject::new();
    entries.insert(
        JsonText::new("tool_id"),
        CanonicalValue::text(descriptor.tool_id()),
    );
    entries.insert(JsonText::new("version"), CanonicalValue::text(version));
    entries.insert(
        JsonText::new("name"),
        CanonicalValue::text(descriptor.name().as_str().unwrap_or_default()),
    );
    entries.insert(
        JsonText::new("description"),
        CanonicalValue::text(descriptor.description().as_str().unwrap_or_default()),
    );
    entries.insert(
        JsonText::new("input_schema_digest"),
        CanonicalValue::text(descriptor.input_schema_digest().to_hex()),
    );
    entries.insert(
        JsonText::new("output_schema_digest"),
        CanonicalValue::text(descriptor.output_schema_digest().to_hex()),
    );
    entries.insert(
        JsonText::new("side_effect_level"),
        CanonicalValue::text(descriptor.side_effect_level().as_str()),
    );
    entries.insert(
        JsonText::new("required_capabilities"),
        CanonicalValue::array(
            descriptor
                .required_capabilities()
                .iter()
                .map(|capability| CanonicalValue::text(capability.as_str())),
        ),
    );
    entries.insert(
        JsonText::new("timeout_seconds"),
        CanonicalValue::Float(descriptor.timeout_seconds()),
    );
    entries.insert(
        JsonText::new("destructive"),
        CanonicalValue::Bool(descriptor.destructive()),
    );
    entries.insert(
        JsonText::new("supports_structured_output"),
        CanonicalValue::Bool(descriptor.supports_structured_output()),
    );
    entries.insert(
        JsonText::new("idempotent"),
        CanonicalValue::Bool(descriptor.idempotent()),
    );
    ToolDescriptor::from_value(&CanonicalValue::Object(entries)).expect("descriptor")
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// The corpus of the differential.
fn corpus() -> Vec<Case> {
    let mut cases = curated();
    cases.extend(randomised(0x5eed_2026_1019, 120));
    cases
}

#[test]
fn the_registry_matches_live_python() {
    let cases = corpus();
    let document = CanonicalValue::array(cases.iter().map(Case::to_value).collect::<Vec<_>>());
    let oracle = run_oracle("registry", &document, None);

    assert_eq!(oracle.len(), cases.len(), "one oracle result per case");
    let mut comparisons = 0usize;
    let mut failures = 0usize;
    for (index, case) in cases.iter().enumerate() {
        let native = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| native_case(case)))
            .unwrap_or_else(|_| panic!("case {index} panicked natively"));
        let oracle_case = &oracle[index];
        let CanonicalValue::Object(oracle_case) = oracle_case else {
            panic!("oracle case is an object");
        };
        let Some(CanonicalValue::Array(oracle_operations)) =
            oracle_case.get(&JsonText::new("operations"))
        else {
            panic!("oracle case has operations");
        };
        let native_operations = match native {
            CanonicalValue::Object(ref entries) => {
                match entries.get(&JsonText::new("operations")) {
                    Some(CanonicalValue::Array(items)) => items.clone(),
                    _ => panic!("native case has operations"),
                }
            }
            _ => panic!("native case is an object"),
        };
        assert_eq!(
            native_operations.len(),
            oracle_operations.len(),
            "case {index} has the same number of operations"
        );
        for (position, (native, oracle)) in native_operations
            .iter()
            .zip(oracle_operations.iter())
            .enumerate()
        {
            comparisons += 1;
            if let CanonicalValue::Object(entries) = native {
                if matches!(
                    entries.get(&JsonText::new("ok")),
                    Some(CanonicalValue::Bool(false))
                ) {
                    failures += 1;
                }
            }
            assert_same(
                native,
                oracle,
                &format!("registry case {index} operation {position}"),
            );
        }
    }
    // The corpus exercises the refusal paths as well as the accepted ones.
    assert!(comparisons >= 400, "{comparisons} comparisons");
    assert!(failures >= 40, "{failures} refusals");
}

/// Run one case natively.
fn native_case(case: &Case) -> CanonicalValue {
    let descriptor = match descriptor_value(case) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            return CanonicalValue::object([
                ("ok", CanonicalValue::Bool(false)),
                ("error", CanonicalValue::text("ContractError")),
                ("message", CanonicalValue::text(error)),
            ]);
        }
    };
    let handler = Arc::new(TestHandler::new(descriptor.clone(), case.first_party));
    let registry = ToolRegistry::new();
    let mut built: Vec<RegisteredTool> = Vec::new();
    let mut outcomes: Vec<CanonicalValue> = Vec::new();
    for operation in &case.operations {
        let outcome = native_operation_full(
            &registry,
            &descriptor,
            &handler,
            &case.input_schema,
            &case.output_schema,
            &mut built,
            operation,
        );
        outcomes.push(match outcome {
            Ok(value) => {
                CanonicalValue::object([("ok", CanonicalValue::Bool(true)), ("output", value)])
            }
            Err(error) => CanonicalValue::object([
                ("ok", CanonicalValue::Bool(false)),
                ("error", CanonicalValue::text(error.class_name())),
                ("message", CanonicalValue::text(error.message())),
            ]),
        });
    }
    CanonicalValue::object([("operations", CanonicalValue::array(outcomes))])
}

/// Build the descriptor of a case.
fn descriptor_value(case: &Case) -> Result<ToolDescriptor, String> {
    let input_digest = match &case.input_schema_digest {
        Some(digest) => digest.clone(),
        None => canonical_json_digest(&case.input_schema).map_err(|error| error.to_string())?,
    };
    let output_digest = match &case.output_schema_digest {
        Some(digest) => digest.clone(),
        None => canonical_json_digest(&case.output_schema).map_err(|error| error.to_string())?,
    };
    let mut entries = CanonicalObject::new();
    entries.insert(
        JsonText::new("tool_id"),
        CanonicalValue::text(&case.tool_id),
    );
    entries.insert(
        JsonText::new("version"),
        CanonicalValue::text(&case.version),
    );
    entries.insert(JsonText::new("name"), CanonicalValue::text("Test tool"));
    entries.insert(
        JsonText::new("description"),
        CanonicalValue::text("A test tool."),
    );
    entries.insert(
        JsonText::new("input_schema_digest"),
        CanonicalValue::text(input_digest),
    );
    entries.insert(
        JsonText::new("output_schema_digest"),
        CanonicalValue::text(output_digest),
    );
    entries.insert(
        JsonText::new("side_effect_level"),
        CanonicalValue::text(&case.side_effect_level),
    );
    entries.insert(
        JsonText::new("required_capabilities"),
        CanonicalValue::array(case.required_capabilities.iter().map(CanonicalValue::text)),
    );
    entries.insert(
        JsonText::new("timeout_seconds"),
        CanonicalValue::Float(case.timeout_seconds),
    );
    entries.insert(
        JsonText::new("destructive"),
        CanonicalValue::Bool(case.destructive),
    );
    entries.insert(
        JsonText::new("supports_structured_output"),
        CanonicalValue::Bool(true),
    );
    entries.insert(JsonText::new("idempotent"), CanonicalValue::Bool(true));
    ToolDescriptor::from_value(&CanonicalValue::Object(entries)).map_err(|error| error.to_string())
}

/// Compare a native result with the oracle result, byte for byte.
///
/// The registry classes are compared as well, so a mismatch of the exception
/// class is a failure.
fn assert_same(native: &CanonicalValue, oracle: &CanonicalValue, label: &str) {
    let native_bytes = canonical(native);
    let oracle_bytes = canonical(oracle);
    assert_eq!(
        native_bytes, oracle_bytes,
        "native and oracle differ for {label}\nnative: {native_bytes}\noracle: {oracle_bytes}"
    );
}

/// The corpus sizes are stable, so the report cannot drift away from the
/// corpus that runs.
#[test]
fn the_registry_corpus_sizes_are_stable() {
    let cases = corpus();
    let curated_count = curated().len();
    assert_eq!(cases.len(), curated_count + 120);
    assert!(curated_count >= 25);
}

/// The side effect levels of the corpus are the ones the contract knows.
#[test]
fn the_registry_corpus_uses_known_side_effect_levels() {
    for case in corpus() {
        assert!(SideEffectLevel::parse(&case.side_effect_level).is_some());
    }
}
