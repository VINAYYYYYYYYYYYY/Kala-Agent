//! The live differential of the native tool executor.
//!
//! Every case is executed twice on this host: once by the repository's own
//! Python executor and once by the native executor. The two decisions are
//! compared field by field — the status, the error code, the sanitized
//! diagnostic, the result payload, the artifacts, the provenance record and
//! the three approval digests — and the host process table is checked after
//! each run, so a worker that left a descendant behind is a failure.
//!
//! The corpus covers every misbehaviour entry point of the reference
//! simulator: the crash, the escape, the flood, the hang, the lie, the
//! lingering child, the stray child and the unrecognised error code.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kala_contracts::{ContractModel, ToolRunStatus};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_process::CancelToken;
use kala_tools::executor::{
    ExecutionOptions, ExecutorConfig, IsolationMode, SandboxProfile, ToolExecutor,
};
use kala_tools::registry::{RegistryHandler, ToolRegistry};
use kala_tools::simulator::{
    EVALUATOR_CAPABILITIES, EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION, SIMULATOR_CAPABILITIES,
    SIMULATOR_TOOL_ID, input_schema, output_schema, register_reference_evaluator,
};
use support::{canonical, product_source_root, python_executable, run_oracle};

/// The misbehaviour entry points of the reference simulator.
const CRASH: &str = "kala_agent.tools.simulator:crash_invoke";
const ESCAPE: &str = "kala_agent.tools.simulator:escape_invoke";
const FLOOD: &str = "kala_agent.tools.simulator:flood_invoke";
const HANG: &str = "kala_agent.tools.simulator:hang_invoke";
const LIAR: &str = "kala_agent.tools.simulator:liar_invoke";
const LINGERING: &str = "kala_agent.tools.simulator:lingering_child_invoke";
const STRAY: &str = "kala_agent.tools.simulator:stray_child_invoke";
const UNRECOGNIZED: &str = "kala_agent.tools.simulator:unrecognized_code_invoke";
const ABSENT: &str = "kala_agent.tools.simulator:absent_invoke";
const INVOKE: &str = "kala_agent.tools.simulator:invoke";

/// The name of the file a stray child writes after the call reported.
const STRAY_CHILD_NAME: &str = "stray-child-effect.txt";

/// Build a JSON object.
fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    let mut object = CanonicalObject::new();
    for (key, value) in entries {
        object.insert(JsonText::new(key), value);
    }
    CanonicalValue::Object(object)
}

/// Build a JSON array of strings.
fn strings(values: &[&str]) -> CanonicalValue {
    CanonicalValue::array(values.iter().map(|value| CanonicalValue::text(*value)))
}

/// Build a JSON array of owned strings.
fn owned_strings(values: &[String]) -> CanonicalValue {
    CanonicalValue::array(
        values
            .iter()
            .map(|value| CanonicalValue::text(value.as_str())),
    )
}

/// The input schema of the misbehaviour tools.
fn echo_input() -> CanonicalValue {
    object(vec![
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            object(vec![(
                "value",
                object(vec![
                    ("maxLength", CanonicalValue::Float(32.0)),
                    ("type", CanonicalValue::text("string")),
                ]),
            )]),
        ),
        ("required", strings(&["value"])),
        ("type", CanonicalValue::text("object")),
    ])
}

/// The output schema of the misbehaviour tools.
fn echo_output() -> CanonicalValue {
    object(vec![
        ("additionalProperties", CanonicalValue::Bool(false)),
        (
            "properties",
            object(vec![(
                "value",
                object(vec![("type", CanonicalValue::text("string"))]),
            )]),
        ),
        ("required", strings(&["value"])),
        ("type", CanonicalValue::text("object")),
    ])
}

/// Attach the schemas of a case.
fn with_schemas(
    case: CanonicalValue,
    input_schema: Option<CanonicalValue>,
    output_schema: Option<CanonicalValue>,
) -> CanonicalValue {
    let mut entries = members(&case).clone();
    if let Some(input_schema) = input_schema {
        entries.insert(JsonText::new("input_schema"), input_schema);
    }
    if let Some(output_schema) = output_schema {
        entries.insert(JsonText::new("output_schema"), output_schema);
    }
    CanonicalValue::Object(entries)
}

/// The misbehaviour tool of a case.
fn misbehaviour(entry_point: &str, side_effect: &str, capabilities: &[&str]) -> CanonicalValue {
    object(vec![
        ("kind", CanonicalValue::text("subprocess")),
        ("tool_id", CanonicalValue::text("test.differential")),
        ("entry_point", CanonicalValue::text(entry_point)),
        ("side_effect_level", CanonicalValue::text(side_effect)),
        ("capabilities", strings(capabilities)),
        ("timeout_seconds", CanonicalValue::Float(30.0)),
    ])
}

/// The simulator tool of a case.
fn simulator_tool() -> CanonicalValue {
    object(vec![
        ("kind", CanonicalValue::text("subprocess")),
        ("tool_id", CanonicalValue::text(SIMULATOR_TOOL_ID)),
        ("entry_point", CanonicalValue::text(INVOKE)),
        ("side_effect_level", CanonicalValue::text("local_write")),
        ("capabilities", strings(&SIMULATOR_CAPABILITIES)),
        ("timeout_seconds", CanonicalValue::Float(20.0)),
    ])
}

/// A case of the differential.
fn case(
    tool: CanonicalValue,
    call_id: &str,
    arguments: &str,
    requested: &[&str],
    side_effect: &str,
    timeout_seconds: f64,
    granted: &[&str],
) -> CanonicalValue {
    object(vec![
        ("tool", tool),
        (
            "call",
            object(vec![
                ("call_id", CanonicalValue::text(call_id)),
                ("arguments_json", CanonicalValue::text(arguments)),
                (
                    "requested_capabilities",
                    CanonicalValue::array(
                        requested
                            .iter()
                            .map(|value| CanonicalValue::text(*value))
                            .collect::<Vec<_>>(),
                    ),
                ),
                ("side_effect_level", CanonicalValue::text(side_effect)),
                ("timeout_seconds", CanonicalValue::Float(timeout_seconds)),
            ]),
        ),
        ("granted", strings(granted)),
    ])
}

/// The corpus of the differential.
#[allow(clippy::vec_init_then_push)]
fn corpus() -> Vec<CanonicalValue> {
    let mut cases: Vec<CanonicalValue> = Vec::new();
    // The reference simulator, with and without a report.
    cases.push(case(
        simulator_tool(),
        "call-success",
        "{\"operation\": \"beam_volume\", \"write_report\": true}",
        &SIMULATOR_CAPABILITIES,
        "local_write",
        20.0,
        &SIMULATOR_CAPABILITIES,
    ));
    cases.push(case(
        simulator_tool(),
        "call-read",
        "{\"operation\": \"bar_summary\"}",
        &SIMULATOR_CAPABILITIES,
        "local_write",
        20.0,
        &SIMULATOR_CAPABILITIES,
    ));
    // The refusals of the authorization stage.
    let absent_case = case(
        object(vec![
            ("kind", CanonicalValue::text("subprocess")),
            ("tool_id", CanonicalValue::text("test.present")),
            ("entry_point", CanonicalValue::text(INVOKE)),
            ("side_effect_level", CanonicalValue::text("read")),
            ("capabilities", strings(&[])),
            ("timeout_seconds", CanonicalValue::Float(5.0)),
        ]),
        "call-absent",
        "{}",
        &[],
        "read",
        5.0,
        &[],
    );
    let absent_call = members(&absent_case)
        .get(&JsonText::new("call"))
        .expect("the call")
        .clone();
    let mut absent_call = members(&absent_call).clone();
    absent_call.insert(
        JsonText::new("tool_id"),
        CanonicalValue::text("test.absent"),
    );
    let mut absent = members(&absent_case).clone();
    absent.insert(JsonText::new("call"), CanonicalValue::Object(absent_call));
    cases.push(CanonicalValue::Object(absent));
    cases.push(case(
        simulator_tool(),
        "call-missing-capability",
        "{}",
        &SIMULATOR_CAPABILITIES,
        "local_write",
        5.0,
        &["design.read"],
    ));
    cases.push(case(
        simulator_tool(),
        "call-side-effect",
        "{}",
        &SIMULATOR_CAPABILITIES,
        "read",
        5.0,
        &SIMULATOR_CAPABILITIES,
    ));
    cases.push(with_schemas(
        case(
            misbehaviour(HANG, "read", &["network"]),
            "call-network",
            "{\"value\": \"hello\"}",
            &["network"],
            "read",
            5.0,
            &["network"],
        ),
        Some(echo_input()),
        Some(echo_output()),
    ));
    cases.push(with_schemas(
        case(
            misbehaviour(HANG, "egress", &["network.egress"]),
            "call-egress",
            "{\"value\": \"hello\"}",
            &["network.egress"],
            "egress",
            5.0,
            &["network.egress"],
        ),
        Some(echo_input()),
        Some(echo_output()),
    ));
    // The misbehaviour entry points.
    cases.push(with_schemas(
        case(
            misbehaviour(CRASH, "read", &["design.read"]),
            "call-crash",
            "{\"value\": \"hello\"}",
            &["design.read"],
            "read",
            10.0,
            &["design.read"],
        ),
        Some(echo_input()),
        Some(echo_output()),
    ));
    cases.push(with_schemas(
        case(
            misbehaviour(FLOOD, "read", &["design.read"]),
            "call-flood",
            "{\"value\": \"hello\"}",
            &["design.read"],
            "read",
            20.0,
            &["design.read"],
        ),
        Some(echo_input()),
        Some(echo_output()),
    ));
    // The liar reports a result that does not satisfy the output schema of
    // the reference simulator.
    cases.push(with_schemas(
        case(
            misbehaviour(LIAR, "read", &["design.read"]),
            "call-liar",
            "{\"value\": \"hello\"}",
            &["design.read"],
            "read",
            10.0,
            &["design.read"],
        ),
        Some(echo_input()),
        None,
    ));
    // The escape reports an artifact outside its workspace.
    cases.push(with_schemas(
        case(
            misbehaviour(ESCAPE, "local_write", &["design.read"]),
            "call-escape",
            "{\"value\": \"hello\"}",
            &["design.read"],
            "local_write",
            10.0,
            &["design.read"],
        ),
        Some(echo_input()),
        None,
    ));
    cases.push(with_schemas(
        case(
            misbehaviour(UNRECOGNIZED, "read", &["design.read"]),
            "call-unrecognized",
            "{\"value\": \"hello\"}",
            &["design.read"],
            "read",
            10.0,
            &["design.read"],
        ),
        Some(echo_input()),
        Some(echo_output()),
    ));
    cases.push(with_schemas(
        case(
            misbehaviour(ABSENT, "read", &["design.read"]),
            "call-absent-import",
            "{\"value\": \"hello\"}",
            &["design.read"],
            "read",
            10.0,
            &["design.read"],
        ),
        Some(echo_input()),
        Some(echo_output()),
    ));
    cases.push(with_schemas(
        case(
            misbehaviour(HANG, "read", &["design.read"]),
            "call-hang",
            "{\"value\": \"hello\"}",
            &["design.read"],
            "read",
            2.0,
            &["design.read"],
        ),
        Some(echo_input()),
        Some(echo_output()),
    ));
    cases.push(with_schemas(
        case(
            misbehaviour(STRAY, "read", &["design.read"]),
            "call-stray",
            "{\"value\": \"hello\"}",
            &["design.read"],
            "read",
            0.8,
            &["design.read"],
        ),
        Some(echo_input()),
        Some(echo_output()),
    ));
    cases.push(with_schemas(
        case(
            misbehaviour(LINGERING, "read", &["design.read"]),
            "call-lingering",
            "{\"value\": \"hello\"}",
            &["design.read"],
            "read",
            10.0,
            &["design.read"],
        ),
        Some(echo_input()),
        Some(echo_output()),
    ));
    // The cancellation of a call that never dispatched.
    let cancelled = case(
        simulator_tool(),
        "call-cancelled",
        "{}",
        &SIMULATOR_CAPABILITIES,
        "local_write",
        5.0,
        &SIMULATOR_CAPABILITIES,
    );
    let mut cancelled = members(&cancelled).clone();
    cancelled.insert(JsonText::new("cancel"), CanonicalValue::Bool(true));
    cases.push(CanonicalValue::Object(cancelled));
    // The trusted in-process evaluator.
    let evaluator = case(
        object(vec![
            ("kind", CanonicalValue::text("evaluator")),
            ("tool_id", CanonicalValue::text(EVALUATOR_TOOL_ID)),
            ("entry_point", CanonicalValue::text(INVOKE)),
            ("side_effect_level", CanonicalValue::text("none")),
            ("capabilities", strings(&EVALUATOR_CAPABILITIES)),
            ("timeout_seconds", CanonicalValue::Float(10.0)),
        ]),
        "call-evaluator",
        "{\"operation\": \"beam_volume\"}",
        &EVALUATOR_CAPABILITIES,
        "none",
        10.0,
        &[],
    );
    let mut evaluator = members(&evaluator).clone();
    evaluator.insert(
        JsonText::new("isolation"),
        CanonicalValue::text("trusted_in_process_evaluator"),
    );
    cases.push(CanonicalValue::Object(evaluator));
    // The oversized arguments.
    let oversized = case(
        simulator_tool(),
        "call-oversized",
        &format!("{{\"operation\": \"{}\"}}", "x".repeat(4_096)),
        &SIMULATOR_CAPABILITIES,
        "local_write",
        5.0,
        &SIMULATOR_CAPABILITIES,
    );
    let mut oversized = members(&oversized).clone();
    oversized.insert(
        JsonText::new("profile"),
        object(vec![("max_argument_bytes", CanonicalValue::Float(128.0))]),
    );
    cases.push(CanonicalValue::Object(oversized));
    // The saturated worker pool.
    let saturated = case(
        misbehaviour(HANG, "read", &["design.read"]),
        "call-saturated",
        "{\"value\": \"hello\"}",
        &["design.read"],
        "read",
        2.0,
        &["design.read"],
    );
    let mut saturated = members(&saturated).clone();
    saturated.insert(
        JsonText::new("profile"),
        object(vec![
            ("max_workers", CanonicalValue::Float(1.0)),
            ("max_queue_seconds", CanonicalValue::Float(0.05)),
            ("default_timeout_seconds", CanonicalValue::Float(10.0)),
        ]),
    );
    saturated.insert(JsonText::new("input_schema"), echo_input());
    saturated.insert(JsonText::new("output_schema"), echo_output());
    cases.push(CanonicalValue::Object(saturated));
    // The policy approval: a call that needs an approval, and the same call
    // after the approval was recorded.
    for (call_id, approve) in [("call-approval-required", false), ("call-approved", true)] {
        let policy_case = case(
            simulator_tool(),
            call_id,
            "{\"operation\": \"beam_volume\"}",
            &SIMULATOR_CAPABILITIES,
            "local_write",
            20.0,
            &SIMULATOR_CAPABILITIES,
        );
        let mut policy_case = members(&policy_case).clone();
        policy_case.insert(JsonText::new("policy"), strings(&SIMULATOR_CAPABILITIES));
        policy_case.insert(JsonText::new("approve"), CanonicalValue::Bool(approve));
        cases.push(CanonicalValue::Object(policy_case));
    }
    // The strict profile.
    let strict = case(
        simulator_tool(),
        "call-strict",
        "{\"operation\": \"beam_volume\"}",
        &SIMULATOR_CAPABILITIES,
        "local_write",
        20.0,
        &SIMULATOR_CAPABILITIES,
    );
    let mut strict = members(&strict).clone();
    strict.insert(
        JsonText::new("profile"),
        object(vec![(
            "isolation_profile",
            CanonicalValue::text("strict_linux"),
        )]),
    );
    cases.push(CanonicalValue::Object(strict));
    cases
}

/// The members of an object.
fn members(value: &CanonicalValue) -> &CanonicalObject {
    let CanonicalValue::Object(entries) = value else {
        panic!("the value is not an object: {value:?}");
    };
    entries
}

/// A text member.
fn text(value: &CanonicalValue, key: &str) -> String {
    match members(value).get(&JsonText::new(key)) {
        Some(CanonicalValue::Text(text)) => text.to_string_lossy().to_owned(),
        _ => panic!("the member {key} is not a string"),
    }
}

/// A text member with a default.
fn text_or(value: &CanonicalValue, key: &str, default: &str) -> String {
    match members(value).get(&JsonText::new(key)) {
        Some(CanonicalValue::Text(text)) => text.to_string_lossy().to_owned(),
        _ => default.to_owned(),
    }
}

/// A numeric member with a default.
fn number_or(value: &CanonicalValue, key: &str, default: f64) -> f64 {
    match members(value).get(&JsonText::new(key)) {
        Some(CanonicalValue::Integer(number)) => {
            use num_traits::ToPrimitive;
            number.to_f64().unwrap_or(default)
        }
        Some(CanonicalValue::Float(number)) => *number,
        _ => default,
    }
}

/// A boolean member with a default.
fn flag_or(value: &CanonicalValue, key: &str, default: bool) -> bool {
    match members(value).get(&JsonText::new(key)) {
        Some(CanonicalValue::Bool(value)) => *value,
        _ => default,
    }
}

/// A string list member with a default.
fn list_or(value: &CanonicalValue, key: &str) -> Vec<String> {
    match members(value).get(&JsonText::new(key)) {
        Some(CanonicalValue::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                CanonicalValue::Text(text) => Some(text.to_string_lossy().to_owned()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The profile of a case.
fn profile_of(case: &CanonicalValue) -> SandboxProfile {
    let mut profile = SandboxProfile::new();
    let Some(spec) = members(case).get(&JsonText::new("profile")) else {
        return profile;
    };
    let spec = spec.clone();
    profile = profile
        .with_max_output_bytes(number_or(&spec, "max_output_bytes", 262_144.0) as usize)
        .with_max_argument_bytes(number_or(&spec, "max_argument_bytes", 262_144.0) as usize)
        .with_max_artifact_bytes(number_or(&spec, "max_artifact_bytes", 8_388_608.0) as u64)
        .with_default_timeout_seconds(number_or(&spec, "default_timeout_seconds", 30.0))
        .with_max_workers(number_or(&spec, "max_workers", 4.0) as usize)
        .with_max_queue_seconds(number_or(&spec, "max_queue_seconds", 5.0))
        .with_allow_network(flag_or(&spec, "allow_network", false))
        .with_allow_egress(flag_or(&spec, "allow_egress", false))
        .with_allow_privileged_capabilities(flag_or(&spec, "allow_privileged_capabilities", false))
        .with_enforce_child_limits(flag_or(&spec, "enforce_child_limits", true));
    if text_or(&spec, "isolation_profile", "portable") == "strict_linux" {
        profile = profile.with_isolation_profile(kala_sandbox::IsolationProfile::StrictLinux);
    }
    profile
}

/// The input schema of a case.
fn input_schema_of(case: &CanonicalValue) -> CanonicalValue {
    match members(case).get(&JsonText::new("input_schema")) {
        Some(schema) => schema.clone(),
        None => input_schema(),
    }
}

/// The output schema of a case.
fn output_schema_of(case: &CanonicalValue) -> CanonicalValue {
    match members(case).get(&JsonText::new("output_schema")) {
        Some(schema) => schema.clone(),
        None => output_schema(),
    }
}

/// The descriptor of the tool of a case.
fn descriptor_of(case: &CanonicalValue, tool: &CanonicalValue) -> kala_contracts::ToolDescriptor {
    use kala_tools::protocol::canonical_json_digest;
    let mut entries = CanonicalObject::new();
    entries.insert(
        JsonText::new("tool_id"),
        CanonicalValue::text(text(tool, "tool_id")),
    );
    entries.insert(
        JsonText::new("version"),
        CanonicalValue::text(text_or(tool, "version", "1.0.0")),
    );
    entries.insert(
        JsonText::new("name"),
        CanonicalValue::text(text_or(tool, "name", "Differential tool")),
    );
    entries.insert(
        JsonText::new("description"),
        CanonicalValue::text(text_or(
            tool,
            "description",
            "A tool used by the differential.",
        )),
    );
    entries.insert(
        JsonText::new("input_schema_digest"),
        CanonicalValue::text(
            canonical_json_digest(&input_schema_of(case))
                .expect("the input schema digest")
                .as_str(),
        ),
    );
    entries.insert(
        JsonText::new("output_schema_digest"),
        CanonicalValue::text(
            canonical_json_digest(&output_schema_of(case))
                .expect("the output schema digest")
                .as_str(),
        ),
    );
    entries.insert(
        JsonText::new("side_effect_level"),
        CanonicalValue::text(text(tool, "side_effect_level")),
    );
    entries.insert(
        JsonText::new("required_capabilities"),
        owned_strings(&list_or(tool, "capabilities")),
    );
    entries.insert(
        JsonText::new("timeout_seconds"),
        CanonicalValue::Float(number_or(tool, "timeout_seconds", 10.0)),
    );
    entries.insert(JsonText::new("destructive"), CanonicalValue::Bool(false));
    entries.insert(
        JsonText::new("supports_structured_output"),
        CanonicalValue::Bool(true),
    );
    entries.insert(JsonText::new("idempotent"), CanonicalValue::Bool(true));
    kala_contracts::ToolDescriptor::from_value(&CanonicalValue::Object(entries))
        .expect("descriptor")
}

/// The handler of the tool of a case.
fn handler_of(case: &CanonicalValue, tool: &CanonicalValue) -> Arc<dyn RegistryHandler> {
    let descriptor = descriptor_of(case, tool);
    Arc::new(
        kala_tools::protocol::SubprocessToolHandler::new(
            descriptor,
            &text(tool, "entry_point"),
            &["-B", "-s"],
        )
        .expect("the subprocess handler"),
    )
}

/// The call of a case.
fn call_of(case: &CanonicalValue, tool: &CanonicalValue) -> kala_contracts::ToolCall {
    let call = members(case)
        .get(&JsonText::new("call"))
        .expect("the call")
        .clone();
    let mut entries = CanonicalObject::new();
    entries.insert(
        JsonText::new("call_id"),
        CanonicalValue::text(text(&call, "call_id")),
    );
    entries.insert(
        JsonText::new("tool_id"),
        CanonicalValue::text(text_or(&call, "tool_id", &text(tool, "tool_id"))),
    );
    entries.insert(
        JsonText::new("tool_version"),
        CanonicalValue::text(text_or(tool, "version", "1.0.0")),
    );
    entries.insert(
        JsonText::new("arguments_json"),
        CanonicalValue::text(text_or(&call, "arguments_json", "{}")),
    );
    entries.insert(
        JsonText::new("requested_capabilities"),
        owned_strings(&list_or(&call, "requested_capabilities")),
    );
    entries.insert(
        JsonText::new("side_effect_level"),
        CanonicalValue::text(text(&call, "side_effect_level")),
    );
    entries.insert(
        JsonText::new("timeout_seconds"),
        CanonicalValue::Float(number_or(&call, "timeout_seconds", 20.0)),
    );
    entries.insert(
        JsonText::new("issued_at"),
        CanonicalValue::text("2026-05-06T07:08:09+00:00"),
    );
    kala_contracts::ToolCall::from_value(&CanonicalValue::Object(entries)).expect("the tool call")
}

/// Run one case through the native executor and report the decision.
fn run_case(case: &CanonicalValue, run_directory: &Path) -> CanonicalValue {
    let tool = members(case)
        .get(&JsonText::new("tool"))
        .expect("the tool")
        .clone();
    let registry = Arc::new(ToolRegistry::new());
    if text_or(&tool, "kind", "subprocess") == "evaluator" {
        register_reference_evaluator(&registry, EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION)
            .expect("the evaluator is registered");
    } else {
        let descriptor = descriptor_of(case, &tool);
        registry
            .register(
                descriptor,
                handler_of(case, &tool),
                input_schema_of(case),
                output_schema_of(case),
            )
            .expect("the tool is registered");
    }
    let granted: BTreeSet<String> = list_or(case, "granted").into_iter().collect();
    let policy = match members(case).get(&JsonText::new("policy")) {
        None => None,
        Some(_) => {
            let engine = kala_kernel::policy::PolicyEngine::from_capabilities(
                list_or(case, "policy")
                    .iter()
                    .map(|value| kala_contracts::Text::new(value.as_str())),
            )
            .expect("the policy engine");
            Some(Arc::new(engine))
        }
    };
    let mut config = ExecutorConfig::new(registry, run_directory.to_path_buf());
    config.granted_capabilities = granted;
    config.policy = policy;
    config.profile = profile_of(case);
    config.isolation = match text_or(case, "isolation", "auto").as_str() {
        "subprocess" => IsolationMode::Subprocess,
        "trusted_in_process_evaluator" => IsolationMode::TrustedInProcessEvaluator,
        _ => IsolationMode::Auto,
    };
    config.import_roots = vec![product_source_root()];
    config.python_executable = Some(python_executable().to_string_lossy().into_owned());
    config.sandbox_probe_helper = Some(PathBuf::from(env!(
        "CARGO_BIN_EXE_kala-tools-sandbox-probe"
    )));
    config.sandbox_init = Some(PathBuf::from(env!("CARGO_BIN_EXE_kala-tools-sandbox-init")));
    config.interpreter_layout = Some(support::python_interpreter_layout());
    let executor = ToolExecutor::new(config).expect("the executor is built");
    let call = call_of(case, &tool);
    if flag_or(case, "approve", false) {
        let engine = executor
            .policy_engine()
            .cloned()
            .expect("the policy engine");
        let request = executor.policy_request(&call).expect("the policy request");
        engine
            .approve(&request, &kala_kernel::policy::DecisionContext::new())
            .expect("the approval");
    }
    let cancel = CancelToken::new();
    if flag_or(case, "cancel", false) {
        cancel.cancel();
    }
    let options = ExecutionOptions::new().with_cancel(Some(&cancel));
    let result = executor.execute(&call, &options);
    let digests = executor.approval_digests(&call).ok();
    let arguments_digest = kala_tools::protocol::sha256_hex_text(
        &kala_tools::protocol::canonical_json_dumps(
            &call.arguments_json().to_value().expect("the arguments"),
            kala_tools::protocol::MAX_JSON_BYTES,
        )
        .expect("the canonical arguments"),
    );
    let reports: Vec<CanonicalValue> = executor
        .reports()
        .iter()
        .map(|report| report.to_payload())
        .collect();
    let artifacts: Vec<CanonicalValue> = result
        .artifacts()
        .iter()
        .map(|artifact| {
            artifact
                .to_wire_value()
                .to_canonical_value()
                .expect("the artifact payload")
        })
        .collect();
    object(vec![
        ("status", CanonicalValue::text(result.status().as_str())),
        (
            "error_code",
            match result.error_code() {
                None => CanonicalValue::Null,
                Some(code) => CanonicalValue::text(code),
            },
        ),
        (
            "error_message",
            match result.error_message() {
                None => CanonicalValue::Null,
                Some(message) => CanonicalValue::text(message.as_str().unwrap_or_default()),
            },
        ),
        (
            "result_json",
            match result.result_json() {
                None => CanonicalValue::Null,
                Some(payload) => CanonicalValue::text(payload.as_str()),
            },
        ),
        ("artifacts", CanonicalValue::Array(artifacts)),
        ("reports", CanonicalValue::Array(reports)),
        (
            "digests",
            object(vec![
                (
                    "arguments_digest",
                    CanonicalValue::text(arguments_digest.as_str()),
                ),
                (
                    "state_digest",
                    match digests.as_ref() {
                        None => CanonicalValue::Null,
                        Some(value) => CanonicalValue::text(value.1.as_str()),
                    },
                ),
                (
                    "action_digest",
                    match digests.as_ref() {
                        None => CanonicalValue::Null,
                        Some(value) => CanonicalValue::text(value.2.as_str()),
                    },
                ),
            ]),
        ),
        (
            "probe",
            object(vec![
                (
                    "available",
                    CanonicalValue::Bool(executor.sandbox_probe().available),
                ),
                (
                    "controls",
                    owned_strings(&executor.sandbox_probe().controls),
                ),
                (
                    "platform",
                    CanonicalValue::text(executor.sandbox_probe().platform.as_str()),
                ),
            ]),
        ),
        (
            "enforced_controls",
            owned_strings(&executor.enforced_controls()),
        ),
    ])
}

/// Replace the volatile values of a record with a stable placeholder.
///
/// The process identifier and the duration of a run are different on every
/// run, so they are compared by their presence and not by their value.
fn normalize(record: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(entries) = record else {
        return record.clone();
    };
    let mut normalized = entries.clone();
    if let Some(CanonicalValue::Array(reports)) = normalized.get(&JsonText::new("reports")) {
        let mut normalized_reports: Vec<CanonicalValue> = Vec::with_capacity(reports.len());
        for report in reports {
            let CanonicalValue::Object(fields) = report else {
                normalized_reports.push(report.clone());
                continue;
            };
            let mut fields = fields.clone();
            if let Some(process_id) = fields.get(&JsonText::new("process_id")) {
                let present = !matches!(process_id, CanonicalValue::Null);
                fields.insert(
                    JsonText::new("process_id"),
                    CanonicalValue::text(if present { "present" } else { "absent" }),
                );
            }
            normalized_reports.push(CanonicalValue::Object(fields));
        }
        normalized.insert(
            JsonText::new("reports"),
            CanonicalValue::Array(normalized_reports),
        );
    }
    CanonicalValue::Object(normalized)
}

/// The identifiers of the processes the host still runs.
fn live_process(pid: u32) -> bool {
    let path = format!("/proc/{pid}/stat");
    let Ok(listing) = std::fs::read_to_string(path) else {
        return false;
    };
    let Some(state) = listing.rsplit_once(')').map(|(_, tail)| tail) else {
        return false;
    };
    !state.trim_start().starts_with('Z')
}

/// The process identifiers of the reports of a record.
fn report_processes(record: &CanonicalValue) -> Vec<u32> {
    let CanonicalValue::Object(entries) = record else {
        return Vec::new();
    };
    let Some(CanonicalValue::Array(reports)) = entries.get(&JsonText::new("reports")) else {
        return Vec::new();
    };
    let mut pids: Vec<u32> = Vec::new();
    for report in reports {
        let CanonicalValue::Object(fields) = report else {
            continue;
        };
        if let Some(CanonicalValue::Integer(pid)) = fields.get(&JsonText::new("process_id")) {
            use num_traits::ToPrimitive;
            if let Some(pid) = pid.to_u32() {
                pids.push(pid);
            }
        }
    }
    pids
}

#[test]
fn the_native_executor_agrees_with_the_repository_python_on_every_case() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let cases = corpus();
    let mut prepared: Vec<CanonicalValue> = Vec::with_capacity(cases.len());
    let mut run_directories: Vec<PathBuf> = Vec::with_capacity(cases.len());
    for (index, case) in cases.iter().enumerate() {
        let run_directory = root.path().join(format!("case-{index}")).join("run");
        let mut entries = members(case).clone();
        entries.insert(
            JsonText::new("run_directory"),
            CanonicalValue::text(run_directory.to_string_lossy().as_ref()),
        );
        entries.insert(
            JsonText::new("import_roots"),
            strings(&[product_source_root().to_string_lossy().as_ref()]),
        );
        prepared.push(CanonicalValue::Object(entries));
        run_directories.push(run_directory);
    }
    let oracle = run_oracle("executor", &CanonicalValue::Array(prepared.clone()), None);
    assert_eq!(oracle.len(), prepared.len());
    for (index, case) in prepared.iter().enumerate() {
        let native = run_case(case, &run_directories[index]);
        let python = &oracle[index];
        let CanonicalValue::Object(python_entries) = python else {
            panic!("the oracle record is not an object");
        };
        let ok = matches!(
            python_entries.get(&JsonText::new("ok")),
            Some(CanonicalValue::Bool(true))
        );
        assert!(ok, "case {index} failed in the Python oracle: {python:?}");
        let python_output = python_entries
            .get(&JsonText::new("output"))
            .expect("the oracle output");
        assert_eq!(
            canonical(&normalize(&native)),
            canonical(&normalize(python_output)),
            "case {index} differs between the native executor and the repository Python"
        );
        // The host process table must be clean after the run.
        for pid in report_processes(&native) {
            assert!(
                !live_process(pid),
                "case {index} left the process {pid} running"
            );
        }
    }
}

#[test]
fn a_timed_out_tool_leaves_no_descendant_that_still_writes() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let cases = corpus();
    let index = cases
        .iter()
        .position(|case| {
            members(case)
                .get(&JsonText::new("call"))
                .map(|call| text(call, "call_id") == "call-stray")
                .unwrap_or(false)
        })
        .expect("the stray child case");
    let case = &cases[index];
    let run_directory = root.path().join("stray").join("run");
    let started = std::time::Instant::now();
    let native = run_case(case, &run_directory);
    let elapsed = started.elapsed();
    assert_eq!(
        members(&native).get(&JsonText::new("status")),
        Some(&CanonicalValue::text("timed_out"))
    );
    // The worker was alive when the deadline fired, so the grandchild it left
    // behind really existed and really had to be killed.
    assert!(
        elapsed >= std::time::Duration::from_millis(500),
        "the worker ended before its deadline: {elapsed:?}"
    );
    let workspace = run_directory
        .join("calls")
        .join("call-stray")
        .join("workspace");
    let stray = workspace.join(STRAY_CHILD_NAME);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        assert!(
            !stray.exists(),
            "a timed out tool left a descendant that still writes"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(!stray.exists());
    assert!(!list_partials(&workspace).is_empty() || !stray.exists());
}

#[test]
fn a_successful_tool_leaves_no_descendant_that_still_writes() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let cases = corpus();
    let index = cases
        .iter()
        .position(|case| {
            members(case)
                .get(&JsonText::new("call"))
                .map(|call| text(call, "call_id") == "call-lingering")
                .unwrap_or(false)
        })
        .expect("the lingering child case");
    let case = &cases[index];
    let run_directory = root.path().join("lingering").join("run");
    let native = run_case(case, &run_directory);
    assert_eq!(
        members(&native).get(&JsonText::new("status")),
        Some(&CanonicalValue::text("succeeded"))
    );
    let workspace = run_directory
        .join("calls")
        .join("call-lingering")
        .join("workspace");
    let stray = workspace.join(STRAY_CHILD_NAME);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        assert!(
            !stray.exists(),
            "a successful tool left a descendant that still writes"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(!stray.exists());
}

/// The partial files a workspace holds.
fn list_partials(workspace: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(workspace) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().ends_with(".partial"))
                .unwrap_or(false)
        })
        .collect()
}

/// The environment of the tests is not the environment of the product.
#[test]
fn the_test_harness_uses_the_repository_interpreter() {
    let mut environment: BTreeMap<String, String> = BTreeMap::new();
    environment.insert("TZ".to_owned(), "UTC".to_owned());
    assert_eq!(environment.get("TZ").map(String::as_str), Some("UTC"));
}

/// The status of a run is one the kernel knows.
#[test]
fn every_record_reports_a_known_status() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let cases = corpus();
    for (index, case) in cases.iter().enumerate() {
        let run_directory = root.path().join(format!("status-{index}")).join("run");
        let record = run_case(case, &run_directory);
        let status = text(&record, "status");
        assert!(
            ToolRunStatus::parse(&status).is_some(),
            "case {index} reported the unknown status {status}"
        );
    }
}

#[test]
fn the_corpus_reports_the_expected_decisions() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let cases = corpus();
    let mut observed: Vec<(String, String, Option<String>)> = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let run_directory = root.path().join(format!("expected-{index}")).join("run");
        let record = run_case(case, &run_directory);
        let call = members(case).get(&JsonText::new("call")).expect("the call");
        observed.push((
            text(call, "call_id"),
            text(&record, "status"),
            members(&record)
                .get(&JsonText::new("error_code"))
                .and_then(|value| match value {
                    CanonicalValue::Text(text) => Some(text.to_string_lossy().to_owned()),
                    _ => None,
                }),
        ));
    }
    let expected: Vec<(&str, &str, Option<&str>)> = vec![
        ("call-success", "succeeded", None),
        ("call-read", "succeeded", None),
        ("call-absent", "denied", Some("unknown_tool")),
        (
            "call-missing-capability",
            "denied",
            Some("capability_denied"),
        ),
        ("call-side-effect", "denied", Some("side_effect_mismatch")),
        ("call-network", "denied", Some("network_denied")),
        ("call-egress", "denied", Some("egress_denied")),
        ("call-crash", "failed", Some("tool_crashed")),
        ("call-flood", "failed", Some("output_too_large")),
        ("call-liar", "failed", Some("invalid_result")),
        ("call-escape", "failed", Some("artifact_rejected")),
        ("call-unrecognized", "failed", Some("tool_error")),
        ("call-absent-import", "failed", Some("tool_crashed")),
        ("call-hang", "timed_out", Some("timeout")),
        ("call-stray", "timed_out", Some("timeout")),
        ("call-lingering", "succeeded", None),
        ("call-cancelled", "cancelled", None),
        ("call-evaluator", "succeeded", None),
        ("call-oversized", "failed", Some("invalid_arguments")),
        ("call-saturated", "timed_out", Some("timeout")),
        (
            "call-approval-required",
            "denied",
            Some("approval_required"),
        ),
        ("call-approved", "succeeded", None),
        ("call-strict", "succeeded", None),
    ];
    let observed: Vec<(&str, &str, Option<&str>)> = observed
        .iter()
        .map(|(call_id, status, error_code)| {
            (call_id.as_str(), status.as_str(), error_code.as_deref())
        })
        .collect();
    assert_eq!(observed, expected);
}
