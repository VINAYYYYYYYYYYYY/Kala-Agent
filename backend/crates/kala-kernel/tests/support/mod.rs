//! Shared helpers for the kernel integration tests.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use kala_contracts::{
    ApprovalKind, ApprovalStatus, ContractModel, SideEffectLevel, Text, WorkflowDefinition,
    WorkflowStatus,
};
use kala_json::{CanonicalValue, canonical_dumps, parse_strict};
use num_bigint::BigInt;

use kala_kernel::canonical_state_json;
use kala_kernel::planning::{PlanValidationError, PlanValidator};
use kala_kernel::policy::{
    CapabilityGrant, DecisionContext, GrantSpec, PolicyDecision, PolicyEngine, PolicyRequest,
    PolicyState,
};
use kala_kernel::time::KernelTimestamp;
use kala_kernel::workflow::{
    CheckpointContext, CollectingSink, StepAttempt, WorkflowEngine, WorkflowEvent, WorkflowState,
};

pub fn text(value: &str) -> CanonicalValue {
    CanonicalValue::Text(kala_json::JsonText::new(value))
}

pub fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::object(entries)
}

pub fn array(values: Vec<CanonicalValue>) -> CanonicalValue {
    CanonicalValue::Array(values)
}

pub fn int(value: i64) -> CanonicalValue {
    CanonicalValue::Integer(BigInt::from(value))
}

pub fn bool_value(value: bool) -> CanonicalValue {
    CanonicalValue::Bool(value)
}

pub fn get<'a>(value: &'a CanonicalValue, key: &str) -> Option<&'a CanonicalValue> {
    match value {
        CanonicalValue::Object(object) => object.get(&kala_json::JsonText::new(key)),
        _ => None,
    }
}

pub fn required_text(value: &CanonicalValue, key: &str) -> Text {
    get(value, key)
        .and_then(value_text)
        .unwrap_or_else(|| panic!("missing text field {key}"))
}

pub fn optional_text(value: &CanonicalValue, key: &str) -> Option<Text> {
    match get(value, key) {
        None | Some(CanonicalValue::Null) => None,
        Some(value) => Some(value_text(value).unwrap_or_else(|| panic!("bad text field {key}"))),
    }
}

pub fn required_bool(value: &CanonicalValue, key: &str) -> bool {
    get(value, key)
        .and_then(value_bool)
        .unwrap_or_else(|| panic!("missing bool field {key}"))
}

pub fn optional_bool(value: &CanonicalValue, key: &str) -> bool {
    get(value, key).and_then(value_bool).unwrap_or(false)
}

pub fn required_int(value: &CanonicalValue, key: &str) -> BigInt {
    get(value, key)
        .and_then(value_integer)
        .cloned()
        .unwrap_or_else(|| panic!("missing integer field {key}"))
}

pub fn optional_int(value: &CanonicalValue, key: &str) -> Option<BigInt> {
    get(value, key).and_then(value_integer).cloned()
}

pub fn canonical(value: &CanonicalValue) -> String {
    canonical_state_json(value).expect("canonical value")
}

pub fn value_text(value: &CanonicalValue) -> Option<Text> {
    value
        .as_text()
        .map(|text| Text::from_json_text(text.clone()))
}

pub fn value_bool(value: &CanonicalValue) -> Option<bool> {
    match value {
        CanonicalValue::Bool(value) => Some(*value),
        _ => None,
    }
}

pub fn value_integer(value: &CanonicalValue) -> Option<&BigInt> {
    match value {
        CanonicalValue::Integer(value) => Some(value),
        _ => None,
    }
}

pub fn canonical_dumps_compat(value: &CanonicalValue) -> String {
    canonical_dumps(value, &kala_contracts::model_json_limits()).expect("canonical value")
}

pub fn error_value(error: &dyn KernelErrorLike) -> CanonicalValue {
    object(vec![
        ("error", text(error.python_class_name())),
        ("message", text(error.message())),
    ])
}

pub trait KernelErrorLike {
    fn python_class_name(&self) -> &'static str;
    fn message(&self) -> &str;
}

impl KernelErrorLike for kala_kernel::policy::PolicyError {
    fn python_class_name(&self) -> &'static str {
        self.python_class_name()
    }
    fn message(&self) -> &str {
        self.message()
    }
}

impl KernelErrorLike for kala_kernel::workflow::WorkflowError {
    fn python_class_name(&self) -> &'static str {
        self.python_class_name()
    }
    fn message(&self) -> &str {
        self.message()
    }
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("backend")
        .parent()
        .expect("repo")
        .to_path_buf()
}

pub fn python_executable() -> PathBuf {
    if let Ok(value) = std::env::var("KALA_KERNEL_PYTHON") {
        return PathBuf::from(value);
    }
    let candidate = repo_root().join(".venv/bin/python");
    assert!(
        candidate.exists(),
        "live Python oracle missing: {} (set KALA_KERNEL_PYTHON)",
        candidate.display()
    );
    candidate
}

/// Run the test-only Python oracle and return its exact stdout bytes.
pub fn run_oracle(mode: &str, document: &CanonicalValue) -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle.py");
    let mut command = Command::new(python_executable());
    command
        .arg(script)
        .arg(mode)
        .env(
            "PYTHONPATH",
            repo_root().join("src").to_string_lossy().to_string(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn Python oracle");
    let payload = canonical(document);
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write oracle input");
    let output = child.wait_with_output().expect("wait for Python oracle");
    assert!(
        output.status.success(),
        "Python oracle failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("oracle stdout is UTF-8")
}

/// Compare the native canonical value against the oracle's exact bytes.
pub fn assert_exact_oracle(native: &CanonicalValue, oracle: &str) {
    let native_bytes = canonical(native);
    if native_bytes != oracle {
        let parsed = parse_strict(oracle.as_bytes(), &kala_contracts::model_json_limits())
            .expect("oracle output is JSON");
        let _ = parsed;
        panic!("native/oracle canonical bytes differ\nnative:\n{native_bytes}\noracle:\n{oracle}");
    }
}

// ---------------------------------------------------------------------------
// Policy scenario execution.
// ---------------------------------------------------------------------------

pub fn policy_request(
    value: &CanonicalValue,
) -> Result<PolicyRequest, kala_kernel::policy::PolicyError> {
    let capabilities = match get(value, "requested_capabilities") {
        Some(CanonicalValue::Array(values)) => {
            values.iter().filter_map(value_text).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    let side_effect = required_text(value, "side_effect_level");
    let side_effect =
        SideEffectLevel::parse(&side_effect.to_string_lossy()).unwrap_or(SideEffectLevel::None);
    let checkpoint = optional_text(value, "checkpoint_type")
        .and_then(|value| ApprovalKind::parse(&value.to_string_lossy()));
    PolicyRequest::from_parts(
        required_text(value, "request_id"),
        required_text(value, "state_digest"),
        required_text(value, "action_digest"),
        capabilities,
        side_effect,
        optional_bool(value, "destructive"),
        optional_bool(value, "egress"),
        optional_text(value, "scope"),
        checkpoint,
    )
}

fn decision_context(op: &CanonicalValue) -> DecisionContext {
    let mut context = DecisionContext::new();
    if let Some(value) = optional_text(op, "state_digest") {
        context = context.with_state_digest(value);
    }
    if let Some(value) = optional_text(op, "action_digest") {
        context = context.with_action_digest(value);
    }
    if let Some(value) = optional_text(op, "actor_id") {
        context = context.with_actor_id(value);
    }
    if let Some(value) = optional_text(op, "reason") {
        context = context.with_reason(value);
    }
    if let Some(value) = optional_text(op, "now") {
        context = context
            .with_now(KernelTimestamp::parse_isoformat(&value.to_string_lossy()).expect("now"));
    }
    context
}

fn policy_error(error: &kala_kernel::policy::PolicyError) -> CanonicalValue {
    error_value(error)
}

pub fn policy_native(document: &CanonicalValue) -> CanonicalValue {
    let grant_specs = match get(document, "grants") {
        Some(CanonicalValue::Array(values)) => values
            .iter()
            .map(|item| match item {
                CanonicalValue::Text(value) => {
                    GrantSpec::Capability(Text::from_json_text(value.clone()))
                }
                CanonicalValue::Object(_) => {
                    let capability = required_text(item, "capability");
                    let scope = optional_text(item, "scope");
                    let expires_at = optional_text(item, "expires_at").map(|value| {
                        KernelTimestamp::parse_isoformat(&value.to_string_lossy()).expect("expiry")
                    });
                    GrantSpec::Grant(
                        CapabilityGrant::from_parts(capability, scope, expires_at).expect("grant"),
                    )
                }
                _ => panic!("bad grant"),
            })
            .collect(),
        _ => Vec::new(),
    };
    let clock = optional_text(document, "now")
        .map(|value| KernelTimestamp::parse_isoformat(&value.to_string_lossy()).expect("now"));
    let mut engine = PolicyEngine::new()
        .with_clock(clock)
        .with_grants(grant_specs)
        .expect("grants");
    if let Some(limit) = optional_int(document, "max_tracked_requests") {
        engine = engine.with_max_tracked_requests(limit).expect("limit");
    }
    let mut results: Vec<CanonicalValue> = Vec::new();
    let ops = match get(document, "ops") {
        Some(CanonicalValue::Array(values)) => values,
        _ => {
            return object(vec![("results", array(results))]);
        }
    };
    for op in ops {
        let name = required_text(op, "op").to_string_lossy();
        let result = match name.as_str() {
            "decide" => {
                let request =
                    policy_request(get(op, "request").expect("request")).expect("request");
                let now = optional_text(op, "now").map(|value| {
                    KernelTimestamp::parse_isoformat(&value.to_string_lossy()).expect("now")
                });
                engine
                    .decide(&request, now)
                    .map(|decision| decision.as_value())
                    .unwrap_or_else(|error| policy_error(&error))
            }
            "request_approval" => {
                let request =
                    policy_request(get(op, "request").expect("request")).expect("request");
                let now = optional_text(op, "now").map(|value| {
                    KernelTimestamp::parse_isoformat(&value.to_string_lossy()).expect("now")
                });
                engine
                    .request_approval(&request, now)
                    .map(|record| record.as_value())
                    .unwrap_or_else(|error| policy_error(&error))
            }
            "approve" | "reject" | "abort" | "invalidate" => {
                let context = decision_context(op);
                let result = if let Some(request) = get(op, "request") {
                    let request = policy_request(request).expect("request");
                    match name.as_str() {
                        "approve" => engine.approve(&request, &context),
                        "reject" => engine.reject(&request, &context),
                        "abort" => engine.abort(&request, &context),
                        "invalidate" => engine.invalidate(&request, &context),
                        _ => unreachable!(),
                    }
                } else {
                    let request_id = required_text(op, "request_id");
                    match name.as_str() {
                        "approve" => engine.approve_by_id(&request_id, &context),
                        "reject" => engine.reject_by_id(&request_id, &context),
                        "abort" => engine.abort_by_id(&request_id, &context),
                        "invalidate" => engine.invalidate_by_id(&request_id, &context),
                        _ => unreachable!(),
                    }
                };
                result
                    .map(|decision| decision.as_value())
                    .unwrap_or_else(|error| policy_error(&error))
            }
            "get_approval" => {
                let state_digest = optional_text(op, "state_digest");
                let action_digest = optional_text(op, "action_digest");
                engine
                    .get_approval(
                        &required_text(op, "request_id"),
                        state_digest.as_ref(),
                        action_digest.as_ref(),
                    )
                    .map(|record| record.map_or(CanonicalValue::Null, |record| record.as_value()))
                    .unwrap_or_else(|error| policy_error(&error))
            }
            "forget_tool_class" => {
                let scope = optional_text(op, "scope");
                engine
                    .forget_tool_class(&required_text(op, "action_digest"), scope.as_ref())
                    .map(CanonicalValue::Bool)
                    .unwrap_or_else(|error| policy_error(&error))
            }
            "grant" => {
                let grant = match get(op, "grant").expect("grant") {
                    CanonicalValue::Text(value) => {
                        CapabilityGrant::new(Text::from_json_text(value.clone())).expect("grant")
                    }
                    item => CapabilityGrant::from_parts(
                        required_text(item, "capability"),
                        optional_text(item, "scope"),
                        optional_text(item, "expires_at").map(|value| {
                            KernelTimestamp::parse_isoformat(&value.to_string_lossy())
                                .expect("expiry")
                        }),
                    )
                    .expect("grant"),
                };
                engine.grant(grant).as_value()
            }
            "revoke" => engine
                .revoke(&required_text(op, "capability"))
                .map(CanonicalValue::Bool)
                .unwrap_or_else(|error| policy_error(&error)),
            "export_state" => engine.export_state().as_value(),
            "import_state" => engine
                .import_state(get(op, "state").expect("state"))
                .map(|_| CanonicalValue::Null)
                .unwrap_or_else(|error| policy_error(&error)),
            "clock" => engine
                .clock()
                .map_or(CanonicalValue::Null, |value| text(&value.isoformat())),
            "grants" => array(
                engine
                    .grants()
                    .into_iter()
                    .map(|value| text(&value.to_string_lossy()))
                    .collect(),
            ),
            "max_tracked_requests" => CanonicalValue::Integer(engine.max_tracked_requests()),
            "tracked_request_ids" => array(
                engine
                    .tracked_request_ids()
                    .into_iter()
                    .map(|value| text(&value.to_string_lossy()))
                    .collect(),
            ),
            "approvals" => array(
                engine
                    .approvals()
                    .iter()
                    .map(|value| value.as_value())
                    .collect(),
            ),
            "approved_tool_classes" => array(
                engine
                    .approved_tool_classes()
                    .into_iter()
                    .map(|value| text(&value.to_string_lossy()))
                    .collect(),
            ),
            other => panic!("unknown policy op {other}"),
        };
        results.push(result);
    }
    object(vec![("results", array(results))])
}

// ---------------------------------------------------------------------------
// Workflow scenario execution.
// ---------------------------------------------------------------------------

fn workflow_definition(document: &CanonicalValue) -> WorkflowDefinition {
    WorkflowDefinition::from_value(get(document, "definition").expect("definition"))
        .expect("definition")
}

fn workflow_state(value: &CanonicalValue) -> WorkflowState {
    match value {
        CanonicalValue::Text(text) => {
            WorkflowState::from_json(&text.to_string_lossy()).expect("workflow state")
        }
        _ => WorkflowState::from_value(value).expect("workflow state"),
    }
}

fn checkpoint_context(op: &CanonicalValue) -> CheckpointContext {
    let mut context = CheckpointContext::new();
    if let Some(value) = optional_text(op, "state_digest") {
        context = context.with_state_digest(value);
    }
    if let Some(value) = optional_text(op, "action_digest") {
        context = context.with_action_digest(value);
    }
    if let Some(value) = optional_text(op, "actor_id") {
        context = context.with_actor_id(value);
    }
    if let Some(value) = optional_text(op, "reason") {
        context = context.with_reason(value);
    }
    if let Some(value) = optional_text(op, "now") {
        context = context
            .with_now(KernelTimestamp::parse_isoformat(&value.to_string_lossy()).expect("now"));
    }
    context
}

pub fn workflow_native(document: &CanonicalValue) -> CanonicalValue {
    let definition = workflow_definition(document);
    let sink = Arc::new(CollectingSink::new());
    let mut engine = WorkflowEngine::new(definition).with_sink(sink.clone());
    if let Some(clock) = optional_text(document, "clock") {
        engine = engine.with_clock(Some(
            KernelTimestamp::parse_isoformat(&clock.to_string_lossy()).expect("clock"),
        ));
    }
    if let Some(CanonicalValue::Object(values)) = get(document, "max_attempts") {
        let mut bounds = Vec::new();
        for (key, value) in values {
            bounds.push((
                Text::from_json_text(key.clone()),
                value_integer(value)
                    .cloned()
                    .unwrap_or_else(|| BigInt::from(0)),
            ));
        }
        engine = engine.with_max_attempts(bounds).expect("attempt bounds");
    }
    let mut results: Vec<CanonicalValue> = Vec::new();
    let mut event_cursor = 0usize;
    let ops = match get(document, "ops") {
        Some(CanonicalValue::Array(values)) => values,
        _ => {
            return object(vec![
                ("events", array(Vec::new())),
                ("results", array(results)),
            ]);
        }
    };
    for op in ops {
        let name = required_text(op, "op").to_string_lossy();
        let explicit_state =
            get(op, "state").filter(|value| !matches!(value, CanonicalValue::Null));
        let state = if name == "restore" {
            None
        } else {
            explicit_state.map(workflow_state)
        };
        let mut special_result: Option<CanonicalValue> = None;
        let result_state = match name.as_str() {
            "start" => {
                let workflow_id =
                    optional_text(op, "workflow_id").unwrap_or_else(|| Text::new("workflow-1"));
                let replanning = optional_int(op, "replanning_attempts");
                engine.start(&workflow_id, replanning.as_ref())
            }
            "run" => engine.run(
                state.as_ref(),
                optional_text(op, "transition_id").as_ref(),
                optional_text(op, "step_id").as_ref(),
                get(op, "succeeded").and_then(value_bool),
            ),
            "record_step_result" => {
                let current = state.clone().or_else(|| engine.state()).expect("state");
                engine.record_step_result(
                    &current,
                    &required_text(op, "step_id"),
                    required_bool(op, "succeeded"),
                    optional_text(op, "transition_id").as_ref(),
                    optional_text(op, "error_code").as_ref(),
                )
            }
            "record_step_success" => {
                let current = state.clone().or_else(|| engine.state()).expect("state");
                engine.record_step_success(
                    &current,
                    &required_text(op, "step_id"),
                    optional_text(op, "transition_id").as_ref(),
                )
            }
            "record_step_failure" => {
                let current = state.clone().or_else(|| engine.state()).expect("state");
                engine.record_step_failure(
                    &current,
                    &required_text(op, "step_id"),
                    optional_text(op, "error_code").as_ref(),
                    optional_text(op, "transition_id").as_ref(),
                )
            }
            "resume" => engine.resume(state.as_ref()),
            "approve_checkpoint" => engine.approve_checkpoint(
                state.as_ref(),
                optional_text(op, "approval_id").as_ref(),
                &checkpoint_context(op),
            ),
            "reject_checkpoint" => engine.reject_checkpoint(
                state.as_ref(),
                optional_text(op, "approval_id").as_ref(),
                &checkpoint_context(op),
            ),
            "abort_checkpoint" => engine.abort_checkpoint(
                state.as_ref(),
                optional_text(op, "approval_id").as_ref(),
                &checkpoint_context(op),
            ),
            "resolve_checkpoint" => {
                let decision = required_text(op, "decision");
                engine.resolve_checkpoint_str(
                    state.as_ref(),
                    optional_text(op, "approval_id").as_ref(),
                    &decision.to_string_lossy(),
                    &checkpoint_context(op),
                )
            }
            "decide_checkpoint" => {
                let decision = required_text(op, "decision");
                engine.decide_checkpoint_str(
                    state.as_ref(),
                    optional_text(op, "approval_id").as_ref(),
                    &decision.to_string_lossy(),
                    &checkpoint_context(op),
                )
            }
            "cancel" => engine.cancel(state.as_ref(), optional_text(op, "reason").as_ref()),
            "request_replan" => {
                engine.request_replan(state.as_ref(), optional_text(op, "reason").as_ref())
            }
            "serialize" => {
                let result = engine.serialize(state.as_ref());
                special_result = Some(match result {
                    Ok(value) => text(&value),
                    Err(error) => error_value(&error),
                });
                Ok(state.clone().or_else(|| engine.state()).expect("state"))
            }
            "snapshot" => engine.snapshot(state.as_ref()),
            "state_digest" => {
                let result = engine.state_digest(state.as_ref());
                special_result = Some(match result {
                    Ok(value) => text(&value),
                    Err(error) => error_value(&error),
                });
                Ok(state.clone().or_else(|| engine.state()).expect("state"))
            }
            "restore" => {
                let value = get(op, "state").expect("state");
                match value {
                    CanonicalValue::Text(text) => engine.restore_json(&text.to_string_lossy()),
                    _ => engine.restore_value(value),
                }
            }
            "state" => engine.state().ok_or_else(|| {
                kala_kernel::workflow::WorkflowError::Transition(
                    "workflow has not been started".to_owned(),
                )
            }),
            other => panic!("unknown workflow op {other}"),
        };
        let result = match result_state {
            Ok(state) => state,
            Err(error) => {
                let value = error_value(&error);
                let state_value = engine.state();
                let events = sink.events();
                let since = events[event_cursor..]
                    .iter()
                    .map(WorkflowEvent::as_value)
                    .collect();
                event_cursor = events.len();
                results.push(object(vec![
                    ("result", value),
                    (
                        "state",
                        state_value
                            .as_ref()
                            .map_or(CanonicalValue::Null, WorkflowState::as_value),
                    ),
                    (
                        "state_json",
                        state_value
                            .as_ref()
                            .and_then(|state| state.to_json().ok())
                            .map_or(CanonicalValue::Null, |value| text(&value)),
                    ),
                    ("events_since", array(since)),
                ]));
                continue;
            }
        };
        if let Some(special) = special_result.take() {
            let state_value = engine.state().unwrap_or_else(|| result.clone());
            let events = sink.events();
            let since = events[event_cursor..]
                .iter()
                .map(WorkflowEvent::as_value)
                .collect();
            event_cursor = events.len();
            results.push(object(vec![
                ("result", special),
                ("state", state_value.as_value()),
                ("events_since", array(since)),
            ]));
            continue;
        }
        let result_json = result.to_json().expect("result json");
        let engine_state = engine.state();
        let state_json = engine_state.as_ref().map_or(CanonicalValue::Null, |value| {
            text(&value.to_json().expect("state json"))
        });
        let events = sink.events();
        let since = events[event_cursor..]
            .iter()
            .map(WorkflowEvent::as_value)
            .collect();
        event_cursor = events.len();
        results.push(object(vec![
            ("result", result.as_value()),
            ("result_json", text(&result_json)),
            (
                "state",
                engine_state.map_or(CanonicalValue::Null, |value| value.as_value()),
            ),
            ("state_json", state_json),
            ("events_since", array(since)),
        ]));
    }
    object(vec![
        (
            "events",
            array(sink.events().iter().map(WorkflowEvent::as_value).collect()),
        ),
        ("results", array(results)),
    ])
}

// ---------------------------------------------------------------------------
// Planning scenario execution.
// ---------------------------------------------------------------------------

pub fn planning_native(document: &CanonicalValue) -> CanonicalValue {
    let definition = workflow_definition(document);
    let allowed = get(document, "allowed_capabilities")
        .filter(|value| !matches!(value, CanonicalValue::Null))
        .map(capability_set_from_value);
    let granted = get(document, "granted_capabilities")
        .filter(|value| !matches!(value, CanonicalValue::Null))
        .map(capability_set_from_value);
    let max_steps = optional_int(document, "max_steps").unwrap_or_else(|| BigInt::from(256));
    let max_attempts = optional_int(document, "max_attempts").unwrap_or_else(|| BigInt::from(1));
    let max_total = optional_int(document, "max_total_attempts");
    let validator = PlanValidator::from_config(
        definition,
        allowed,
        granted,
        max_steps,
        max_attempts,
        max_total,
    )
    .expect("validator");
    let mut results = Vec::new();
    if let Some(CanonicalValue::Array(cases)) = get(document, "cases") {
        for case in cases {
            let steps = get(case, "steps").expect("steps");
            let allowed = get(case, "allowed_capabilities")
                .filter(|value| !matches!(value, CanonicalValue::Null))
                .map(capability_set_from_value);
            let granted = get(case, "granted_capabilities")
                .filter(|value| !matches!(value, CanonicalValue::Null))
                .map(capability_set_from_value);
            let replanning = get(case, "replanning_attempts")
                .cloned()
                .unwrap_or_else(|| CanonicalValue::Integer(BigInt::from(0)));
            match validator.validate_values_request(
                steps,
                allowed.as_ref(),
                granted.as_ref(),
                &replanning,
            ) {
                Ok(plan) => {
                    let steps = plan
                        .steps()
                        .iter()
                        .map(|step| step.to_canonical_value().expect("step canonical"))
                        .collect();
                    results.push(object(vec![(
                        "plan",
                        object(vec![
                            (
                                "workflow_definition_id",
                                text(&plan.workflow_definition_id().to_string_lossy()),
                            ),
                            ("steps", array(steps)),
                            (
                                "required_capabilities",
                                array(
                                    plan.required_capabilities()
                                        .iter()
                                        .map(|value| text(&value.to_string_lossy()))
                                        .collect(),
                                ),
                            ),
                            (
                                "max_attempts",
                                CanonicalValue::Integer(plan.max_attempts().clone()),
                            ),
                        ]),
                    )]));
                }
                Err(error) => results.push(planning_error(&error)),
            }
        }
    }
    object(vec![("results", array(results))])
}

fn planning_error(error: &PlanValidationError) -> CanonicalValue {
    error.as_value()
}

fn capability_set_from_value(value: &CanonicalValue) -> kala_kernel::planning::CapabilitySet {
    match value {
        CanonicalValue::Null => kala_kernel::planning::CapabilitySet::new(),
        CanonicalValue::Array(values) => values
            .iter()
            .filter_map(value_text)
            .map(|value| Text::new(&value.to_string_lossy()))
            .collect(),
        _ => panic!("capability set"),
    }
}

// ---------------------------------------------------------------------------
// Scenario builders used by multiple tests.
// ---------------------------------------------------------------------------

pub fn request_value(
    request_id: &str,
    state_digest: &str,
    action_digest: &str,
    capabilities: &[&str],
    side_effect: &str,
    scope: Option<&str>,
    checkpoint: Option<&str>,
) -> CanonicalValue {
    object(vec![
        ("action_digest", text(action_digest)),
        (
            "checkpoint_type",
            checkpoint.map_or(CanonicalValue::Null, text),
        ),
        ("destructive", bool_value(false)),
        ("egress", bool_value(side_effect == "egress")),
        ("request_id", text(request_id)),
        (
            "requested_capabilities",
            array(capabilities.iter().map(|value| text(value)).collect()),
        ),
        ("scope", scope.map_or(CanonicalValue::Null, text)),
        ("side_effect_level", text(side_effect)),
        ("state_digest", text(state_digest)),
    ])
}

pub fn op(name: &str) -> Vec<(&str, CanonicalValue)> {
    vec![("op", text(name))]
}

pub fn simple_definition(checkpoint: bool) -> CanonicalValue {
    object(vec![
        ("available_step_ids", array(vec![text("step-1")])),
        ("definition_id", text("workflow-def")),
        ("initial_state", text("start")),
        ("max_replanning_attempts", int(1)),
        ("name", text("Single step")),
        ("states", array(vec![text("start"), text("done")])),
        (
            "transitions",
            array(vec![object(vec![
                (
                    "checkpoint_type",
                    if checkpoint {
                        text("pre_commit")
                    } else {
                        CanonicalValue::Null
                    },
                ),
                ("from_state", text("start")),
                ("step_id", text("step-1")),
                ("to_state", text("done")),
                ("transition_id", text("transition-1")),
            ])]),
        ),
        ("version", text("1.0.0")),
    ])
}

pub fn three_state_definition() -> CanonicalValue {
    object(vec![
        (
            "available_step_ids",
            array(vec![text("step-1"), text("step-2")]),
        ),
        ("definition_id", text("workflow-def")),
        ("initial_state", text("start")),
        ("max_replanning_attempts", int(2)),
        ("name", text("Branchable")),
        (
            "states",
            array(vec![text("start"), text("middle"), text("done")]),
        ),
        (
            "transitions",
            array(vec![
                object(vec![
                    ("checkpoint_type", CanonicalValue::Null),
                    ("from_state", text("start")),
                    ("step_id", text("step-1")),
                    ("to_state", text("middle")),
                    ("transition_id", text("transition-1")),
                ]),
                object(vec![
                    ("checkpoint_type", CanonicalValue::Null),
                    ("from_state", text("middle")),
                    ("step_id", text("step-2")),
                    ("to_state", text("done")),
                    ("transition_id", text("transition-2")),
                ]),
            ]),
        ),
        ("version", text("1.0.0")),
    ])
}

pub fn step_value(
    step_id: &str,
    catalog_step_id: &str,
    depends_on: &[&str],
    capabilities: &[&str],
    max_attempts: i64,
) -> CanonicalValue {
    object(vec![
        ("catalog_step_id", text(catalog_step_id)),
        (
            "depends_on",
            array(depends_on.iter().map(|value| text(value)).collect()),
        ),
        ("intent", text("Perform a declared step.")),
        ("max_attempts", int(max_attempts)),
        ("parameters_json", text("{}")),
        (
            "required_capabilities",
            array(capabilities.iter().map(|value| text(value)).collect()),
        ),
        ("step_id", text(step_id)),
    ])
}

pub fn expected_oracle() -> bool {
    std::env::var("KALA_KERNEL_SKIP_PYTHON").is_err()
}

pub fn state_digest_of(engine: &WorkflowEngine, state: &WorkflowState) -> String {
    engine.state_digest_of(state).expect("digest")
}

pub fn policy_decision_allowed(decision: &PolicyDecision) -> bool {
    decision.allowed()
}

pub fn approval_status(value: ApprovalStatus) -> &'static str {
    value.as_str()
}

pub fn workflow_status(value: WorkflowStatus) -> &'static str {
    value.as_str()
}

pub fn attempt_value(step_id: &str, attempt: i64, succeeded: bool) -> CanonicalValue {
    StepAttempt::from_parts(Text::new(step_id), BigInt::from(attempt), succeeded, None)
        .expect("attempt")
        .as_value()
}

pub fn policy_state(value: PolicyState) -> CanonicalValue {
    value.as_value()
}

pub fn hash_map() -> HashMap<String, String> {
    HashMap::new()
}
