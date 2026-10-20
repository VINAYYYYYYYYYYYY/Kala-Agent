//! The tool executor.
//!
//! This module is the Rust port of `kala_agent.tools.executor`. It owns the
//! authorization of a tool call, the materialization of the call workspace,
//! the dispatch of the worker, the verification of its answer and the
//! provenance record of the run.
//!
//! # The order of the decisions
//!
//! The order is the security property of the module, so it is reproduced
//! exactly:
//!
//! 1. the tool is resolved (an unknown tool or an unknown version is denied);
//! 2. the approval binding is computed (the argument digest, the envelope
//!    digest, the state digest and the action digest);
//! 3. the call is authorized (capabilities, side effects and policy);
//! 4. a cancelled call never dispatches;
//! 5. the isolation mode is selected (an in-kernel handler is refused);
//! 6. the sandbox verdict is checked (a side effect the profile cannot back
//!    is refused);
//! 7. the workspace is materialized and the arguments are validated again;
//! 8. a worker slot is acquired (a saturated pool denies instead of queueing
//!    forever);
//! 9. the call is dispatched and the answer is verified.
//!
//! # The controls
//!
//! The executor never weakens a sandbox control. The strict profile runs the
//! worker through the accepted native init helper, so the worker is inside a
//! PID namespace with a PID 1 that reaps its orphans; a jail that cannot be
//! established refuses the run instead of running the worker anyway.

mod artifact;
mod dispatch;
mod error;
mod paths;
mod profile;
mod report;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

use kala_budget::{BudgetLedger, BudgetReservation, BudgetUsage};
use kala_contracts::{
    ApprovalKind, ArtifactRef, ContractModel, SideEffectLevel, Timestamp, ToolCall, ToolDescriptor,
    ToolResult, ToolRunStatus,
};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_kernel::policy::{PolicyDecision, PolicyEngine, PolicyReason, PolicyRequest};
use kala_process::CancelToken;
use kala_sandbox::{InterpreterLayout, IsolationProfile, SandboxProbe};

use crate::executor::dispatch::DispatchOutcome;
use crate::executor::paths::{prepare_run_directory, workspace_for, workspace_path};
use crate::protocol::{
    ToolErrorCode, ToolProtocolError, ToolRequestEnvelope, ToolResponseEnvelope,
    canonical_json_digest, canonical_json_dumps, summarize_error,
};
use crate::registry::{RegisteredTool, ToolRegistry, ToolRegistryError};

pub use crate::executor::artifact::{
    BINARY_SIGNATURES, CLASSIFICATION_HEADER_BYTES, TEXTUAL_MEDIA_SUFFIXES, classify_header,
    derive_classification, verify_artifacts,
};
pub use crate::executor::error::{ExecutorResult, ToolExecutorError};
pub use crate::executor::profile::{
    APPROVAL_BINDING_VERSION, EGRESS_CAPABILITIES, EXTRA_ENVIRONMENT_KEYS, FORBIDDEN_CAPABILITIES,
    IsolationMode, MAX_DEFAULT_TIMEOUT_SECONDS, MAX_ERROR_MESSAGE_LENGTH, MAX_QUEUE_SECONDS,
    MAX_WORKERS, MINIMUM_QUEUE_SECONDS, MINIMUM_TIMEOUT_SECONDS, NETWORK_CAPABILITIES,
    PORTABLE_CONTROLS, SandboxProfile, resource_limits_available,
};
pub use crate::executor::report::ToolExecutionReport;

/// The status a refusal is reported with.
fn failure_status(code: ToolErrorCode) -> ToolRunStatus {
    match code {
        ToolErrorCode::Timeout => ToolRunStatus::TimedOut,
        _ => ToolRunStatus::Denied,
    }
}

/// The error codes only the kernel may decide.
fn is_kernel_decision(code: ToolErrorCode) -> bool {
    matches!(
        code,
        ToolErrorCode::ApprovalRequired
            | ToolErrorCode::BudgetExhausted
            | ToolErrorCode::Cancelled
            | ToolErrorCode::CapabilityDenied
            | ToolErrorCode::CapabilityForbidden
            | ToolErrorCode::DescriptorMismatch
            | ToolErrorCode::EgressDenied
            | ToolErrorCode::IsolationRequired
            | ToolErrorCode::NetworkDenied
            | ToolErrorCode::SchemaDigestMismatch
            | ToolErrorCode::SideEffectMismatch
            | ToolErrorCode::Timeout
            | ToolErrorCode::UnknownTool
            | ToolErrorCode::VersionNotFound
            | ToolErrorCode::WorkerCapacityExhausted
    )
}

/// The options of one call.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutionOptions<'a> {
    /// The deadline override of the call.
    ///
    /// The Python implementation accepts the override and does not use it: the
    /// deadline of a call is the smaller of the deadline the call declares and
    /// the deadline of the descriptor, capped by the profile. The port keeps
    /// the parameter and the behaviour, so a caller that relied on the
    /// override is not surprised by a different deadline.
    pub timeout_seconds: Option<f64>,
    /// The cancellation token of the call.
    pub cancel: Option<&'a CancelToken>,
}

impl<'a> ExecutionOptions<'a> {
    /// The default options: no override and no cancellation.
    pub fn new() -> Self {
        Self::default()
    }

    /// A copy with a deadline override.
    pub fn with_timeout_seconds(mut self, value: Option<f64>) -> Self {
        self.timeout_seconds = value;
        self
    }

    /// A copy with a cancellation token.
    pub fn with_cancel(mut self, value: Option<&'a CancelToken>) -> Self {
        self.cancel = value;
        self
    }
}

/// The canonical description a policy approval is bound to.
#[derive(Clone, Debug)]
struct ApprovalBinding {
    envelope: Option<ToolRequestEnvelope>,
    arguments_digest: String,
    workspace: String,
    envelope_digest: String,
    state_digest: String,
    action_digest: String,
    effective_capabilities: Vec<String>,
}

/// A call that cannot be dispatched, with the code the caller must see.
struct CallRejected {
    code: ToolErrorCode,
    message: String,
}

/// The sink that publishes a verified artifact.
///
/// The Python implementation accepts any callable that takes the path of a
/// verified artifact. The return value is not used there either, so the port
/// drops it as well.
pub type ResultSink = Arc<dyn Fn(&Path) + Send + Sync>;

/// The configuration of an executor.
pub struct ExecutorConfig {
    /// The registry the executor resolves the tools in.
    pub registry: Arc<ToolRegistry>,
    /// The run directory of the executor.
    pub run_directory: PathBuf,
    /// The capabilities the executor grants.
    pub granted_capabilities: BTreeSet<String>,
    /// The sandbox profile.
    pub profile: SandboxProfile,
    /// The budget ledger, when the calls are accounted.
    pub ledger: Option<Arc<BudgetLedger>>,
    /// The policy engine, when the calls are authorized by a policy.
    pub policy: Option<Arc<PolicyEngine>>,
    /// The isolation mode.
    pub isolation: IsolationMode,
    /// The import roots the child may read.
    pub import_roots: Vec<PathBuf>,
    /// The interpreter that runs the tool worker.
    pub python_executable: Option<String>,
    /// The sink that publishes a verified artifact.
    pub result_sink: Option<ResultSink>,
    /// The native init helper that builds the strict jail.
    pub sandbox_init: Option<PathBuf>,
    /// The native helper that probes the strict profile.
    pub sandbox_probe_helper: Option<PathBuf>,
    /// The interpreter layout the strict jail binds read-only.
    pub interpreter_layout: Option<InterpreterLayout>,
}

impl ExecutorConfig {
    /// The configuration of an executor with the portable profile.
    pub fn new(registry: Arc<ToolRegistry>, run_directory: impl Into<PathBuf>) -> Self {
        Self {
            registry,
            run_directory: run_directory.into(),
            granted_capabilities: BTreeSet::new(),
            profile: SandboxProfile::new(),
            ledger: None,
            policy: None,
            isolation: IsolationMode::Auto,
            import_roots: Vec::new(),
            python_executable: None,
            result_sink: None,
            sandbox_init: None,
            sandbox_probe_helper: None,
            interpreter_layout: None,
        }
    }
}

/// The executor of the tool calls.
///
/// The executor is safe to share between threads, exactly like the Python
/// executor, which guards its reports with a re-entrant lock.
pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
    run_directory: PathBuf,
    granted: RwLock<BTreeSet<String>>,
    profile: SandboxProfile,
    ledger: Option<Arc<BudgetLedger>>,
    policy: Option<Arc<PolicyEngine>>,
    isolation: IsolationMode,
    import_roots: Vec<PathBuf>,
    python_executable: Option<String>,
    result_sink: Option<ResultSink>,
    sandbox_init: Option<PathBuf>,
    interpreter_layout: InterpreterLayout,
    probe: SandboxProbe,
    probe_controls: std::collections::BTreeSet<String>,
    worker_slots: WorkerSlots,
    reports: Mutex<Vec<ToolExecutionReport>>,
    dispatch_sequence: AtomicU64,
}

impl ToolExecutor {
    /// Build an executor, validating the configuration.
    ///
    /// The run directory is created and confined, the granted capabilities are
    /// normalised and the profile is validated, exactly like the Python
    /// constructor.
    pub fn new(config: ExecutorConfig) -> ExecutorResult<Self> {
        let profile = config.profile.clone().validated()?;
        let run_directory = prepare_run_directory(&config.run_directory)?;
        let granted = normalize_capabilities(config.granted_capabilities.iter())?;
        let import_roots = normalize_import_roots(&config.import_roots)?;
        let probe = probe_sandbox(
            &profile,
            &import_roots,
            config.sandbox_probe_helper.as_deref(),
        );
        let profile = profile.with_sandbox_probe(Some(probe.clone()));
        let interpreter_layout = config
            .interpreter_layout
            .clone()
            .unwrap_or_else(InterpreterLayout::current);
        Ok(Self {
            registry: config.registry,
            run_directory,
            granted: RwLock::new(granted),
            profile: profile.clone(),
            ledger: config.ledger,
            policy: config.policy,
            isolation: config.isolation,
            import_roots,
            python_executable: config.python_executable,
            result_sink: config.result_sink,
            sandbox_init: config.sandbox_init,
            interpreter_layout,
            probe: probe.clone(),
            probe_controls: probe.controls.iter().cloned().collect(),
            worker_slots: WorkerSlots::new(profile.max_workers()),
            reports: Mutex::new(Vec::new()),
            dispatch_sequence: AtomicU64::new(0),
        })
    }

    /// The registry of the executor.
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// The run directory of the executor.
    pub fn run_directory(&self) -> &Path {
        &self.run_directory
    }

    /// The capabilities the executor grants.
    pub fn granted_capabilities(&self) -> BTreeSet<String> {
        self.granted
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// The sandbox profile of the executor.
    pub fn profile(&self) -> &SandboxProfile {
        &self.profile
    }

    /// The policy engine of the executor, when it was built with one.
    pub fn policy_engine(&self) -> Option<&Arc<PolicyEngine>> {
        self.policy.as_ref()
    }

    /// The isolation mode of the executor.
    pub fn isolation(&self) -> IsolationMode {
        self.isolation
    }

    /// The preflight probe of the strict profile.
    pub fn sandbox_probe(&self) -> &SandboxProbe {
        &self.probe
    }

    /// The controls the executor enforces on this host.
    pub fn enforced_controls(&self) -> Vec<String> {
        self.profile.enforced_controls()
    }

    /// The provenance records of the runs, in order.
    pub fn reports(&self) -> Vec<ToolExecutionReport> {
        self.reports
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Register a tool and refuse it when it claims ungranted authority.
    pub fn register(
        &self,
        descriptor: ToolDescriptor,
        handler: Arc<dyn crate::registry::RegistryHandler>,
        input_schema: CanonicalValue,
        output_schema: CanonicalValue,
    ) -> ExecutorResult<RegisteredTool> {
        let tool = self
            .registry
            .build(descriptor, handler, input_schema, output_schema)
            .map_err(|error| ToolExecutorError::executor(error.to_string()))?;
        self.require_authority(&tool)?;
        self.registry
            .register_tool(tool)
            .map_err(|error| ToolExecutorError::executor(error.to_string()))
    }

    /// Refuse a tool that claims authority the executor does not have.
    pub fn require_authority(&self, tool: &RegisteredTool) -> ExecutorResult<RegisteredTool> {
        let granted = self.granted_capabilities();
        let mut missing: Vec<String> = tool
            .required_capabilities()
            .iter()
            .filter(|required| !granted.contains(required.as_str()))
            .map(|required| required.as_str().to_owned())
            .collect();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            return Err(ToolExecutorError::authority(format!(
                "tool {} claims capabilities that are not granted: {}",
                tool.tool_id(),
                missing.join(", ")
            )));
        }
        let required: BTreeSet<String> = tool
            .required_capabilities()
            .iter()
            .map(|value| value.as_str().to_owned())
            .collect();
        self.check_capability_policy(&required)?;
        self.check_side_effect_policy(tool.side_effect_level())?;
        Ok(tool.clone())
    }

    /// Grant a capability and report the new set.
    pub fn grant_capability(&self, capability: &str) -> ExecutorResult<BTreeSet<String>> {
        let normalized = normalize_capabilities(std::iter::once(capability))?;
        let mut granted = self
            .granted
            .write()
            .unwrap_or_else(|error| error.into_inner());
        granted.extend(normalized);
        Ok(granted.clone())
    }

    /// The workspace of a call, without creating anything on disk.
    pub fn workspace_path(&self, call_id: &str) -> ExecutorResult<PathBuf> {
        workspace_path(&self.run_directory, call_id)
    }

    /// The workspace of a call, created and confined to the run directory.
    pub fn workspace_for(&self, call_id: &str) -> ExecutorResult<PathBuf> {
        workspace_for(&self.run_directory, call_id)
    }

    /// Resolve a path inside a workspace.
    pub fn resolve_output_path(
        &self,
        workspace: &Path,
        relative: &str,
    ) -> Result<PathBuf, ToolProtocolError> {
        crate::protocol::resolve_workspace_path(workspace, relative)
    }

    /// The digests an approval of a call is bound to.
    ///
    /// The three digests are the argument digest, the state digest and the
    /// action digest. They are the identity of the approval: a request whose
    /// arguments, workspace, capabilities or tool changed has a different
    /// action digest and cannot be replayed.
    pub fn approval_digests(&self, call: &ToolCall) -> ExecutorResult<(String, String, String)> {
        let tool = self
            .registry
            .require(
                call.tool_id(),
                Some(call.tool_version().as_str().unwrap_or_default()),
            )
            .map_err(|error| ToolExecutorError::executor(error.to_string()))?;
        let binding = self.approval_binding(call, &tool)?;
        Ok((
            binding.arguments_digest,
            binding.state_digest,
            binding.action_digest,
        ))
    }

    /// The policy request of a call.
    pub fn policy_request(&self, call: &ToolCall) -> ExecutorResult<PolicyRequest> {
        let tool = self
            .registry
            .require(
                call.tool_id(),
                Some(call.tool_version().as_str().unwrap_or_default()),
            )
            .map_err(|error| ToolExecutorError::executor(error.to_string()))?;
        let binding = self.approval_binding(call, &tool)?;
        self.request_for(call, &tool, &binding)
    }

    /// Run one tool call.
    ///
    /// The call is refused, failed, timed out or cancelled; it never panics
    /// and it never runs a tool the executor may not run.
    pub fn execute(&self, call: &ToolCall, options: &ExecutionOptions<'_>) -> ToolResult {
        let started_at = now_timestamp();
        let tool = match self.registry.require(
            call.tool_id(),
            Some(call.tool_version().as_str().unwrap_or_default()),
        ) {
            Ok(tool) => tool,
            Err(error) => {
                let code = match &error {
                    ToolRegistryError::VersionNotFound(_) => ToolErrorCode::VersionNotFound,
                    ToolRegistryError::UnknownTool(_) => ToolErrorCode::UnknownTool,
                    _ => ToolErrorCode::SchemaDigestMismatch,
                };
                return self.denied(call, code, &error.to_string(), started_at);
            }
        };
        let binding = match self.approval_binding(call, &tool) {
            Ok(binding) => binding,
            Err(error) => {
                return self.failure(
                    call,
                    ToolRunStatus::Failed,
                    ToolErrorCode::InternalError,
                    error.message(),
                    started_at,
                    0.0,
                );
            }
        };
        if let Some((code, message)) = self.authorize(call, &tool, &binding) {
            return self.denied(call, code, &message, started_at);
        }
        if let Some(cancel) = options.cancel {
            if cancel.is_cancelled() {
                return self.cancelled(
                    call,
                    "the tool call was cancelled before dispatch",
                    started_at,
                    0.0,
                );
            }
        }
        let Some(isolation) = self.select_isolation(&tool) else {
            return self.denied(
                call,
                ToolErrorCode::IsolationRequired,
                &format!(
                    "tool {} cannot be dispatched under the configured isolation policy",
                    tool.tool_id()
                ),
                started_at,
            );
        };
        if let Some(unavailable) = self.sandbox_unavailable(&tool) {
            return self.denied(
                call,
                ToolErrorCode::SandboxUnavailable,
                &unavailable,
                started_at,
            );
        }
        let (workspace, envelope) = match self.prepare_dispatch(call, &tool, &binding) {
            Ok(prepared) => prepared,
            Err(rejected) => {
                return self.failure(
                    call,
                    ToolRunStatus::Failed,
                    rejected.code,
                    &rejected.message,
                    started_at,
                    0.0,
                );
            }
        };
        if !self.worker_slots.acquire(self.profile.max_queue_seconds()) {
            return self.denied(
                call,
                ToolErrorCode::WorkerCapacityExhausted,
                &format!("all {} tool workers are busy", self.profile.max_workers()),
                started_at,
            );
        }
        let outcome = self.dispatch_call(
            call, &tool, &binding, &envelope, &workspace, isolation, options, started_at,
        );
        self.worker_slots.release();
        outcome
    }

    /// Bind an approval to the exact canonical request it authorizes.
    ///
    /// The binding covers the argument digest, the resolved workspace, the
    /// effective capabilities, and the tool, version, descriptor, and schema
    /// digests, so a request id cannot be replayed with different arguments, a
    /// different workspace, or a retargeted tool.
    fn approval_binding(
        &self,
        call: &ToolCall,
        tool: &RegisteredTool,
    ) -> ExecutorResult<ApprovalBinding> {
        let workspace = self.workspace_path(call.call_id())?;
        let deadline = self.profile.timeout_for(
            Some(call.timeout_seconds()),
            tool.descriptor().timeout_seconds(),
        );
        let capabilities = self.effective_capabilities(call, tool);
        let (arguments_digest, envelope_digest, envelope) = match ToolRequestEnvelope::from_call(
            call,
            &workspace,
            deadline,
            &num_bigint::BigInt::from(self.profile.max_output_bytes()),
            &num_bigint::BigInt::from(self.profile.max_artifact_bytes()),
            &capabilities,
        ) {
            Ok(envelope) => (
                envelope.arguments_digest(),
                envelope
                    .envelope_digest()
                    .map_err(|error| ToolExecutorError::executor(error.to_string()))?,
                Some(envelope),
            ),
            Err(_) => {
                let arguments_digest = crate::protocol::sha256_hex_text(&format!(
                    "uncanonicalized:{}",
                    call.arguments_json().as_str()
                ));
                let mut payload = CanonicalObject::new();
                payload.insert(
                    JsonText::new("arguments_digest"),
                    CanonicalValue::text(&arguments_digest),
                );
                payload.insert(
                    JsonText::new("call_id"),
                    CanonicalValue::text(call.call_id()),
                );
                payload.insert(
                    JsonText::new("effective_capabilities"),
                    CanonicalValue::array(
                        capabilities
                            .iter()
                            .map(CanonicalValue::text)
                            .collect::<Vec<_>>(),
                    ),
                );
                payload.insert(
                    JsonText::new("tool_id"),
                    CanonicalValue::text(tool.tool_id()),
                );
                payload.insert(
                    JsonText::new("tool_version"),
                    CanonicalValue::text(tool.version()),
                );
                payload.insert(
                    JsonText::new("workspace"),
                    CanonicalValue::text(posix_text(&workspace)),
                );
                let envelope_digest = canonical_json_digest(&CanonicalValue::Object(payload))
                    .map_err(|error| ToolExecutorError::executor(error.to_string()))?;
                (arguments_digest, envelope_digest, None)
            }
        };
        let granted = self.granted_capabilities();
        let mut state = CanonicalObject::new();
        state.insert(
            JsonText::new("binding_version"),
            CanonicalValue::text(APPROVAL_BINDING_VERSION),
        );
        state.insert(
            JsonText::new("call_id"),
            CanonicalValue::text(call.call_id()),
        );
        state.insert(
            JsonText::new("enforced_controls"),
            CanonicalValue::array(
                self.profile
                    .enforced_controls()
                    .iter()
                    .map(CanonicalValue::text)
                    .collect::<Vec<_>>(),
            ),
        );
        state.insert(
            JsonText::new("granted_capabilities"),
            CanonicalValue::array(granted.iter().map(CanonicalValue::text).collect::<Vec<_>>()),
        );
        state.insert(
            JsonText::new("isolation_mode"),
            CanonicalValue::text(self.isolation.as_str()),
        );
        state.insert(
            JsonText::new("isolation_profile"),
            CanonicalValue::text(self.profile.isolation_profile().as_str()),
        );
        state.insert(
            JsonText::new("profile_name"),
            CanonicalValue::text(self.profile.profile_name()),
        );
        state.insert(
            JsonText::new("tool_descriptor_digest"),
            CanonicalValue::text(tool.descriptor_digest()),
        );
        let state_digest = canonical_json_digest(&CanonicalValue::Object(state))
            .map_err(|error| ToolExecutorError::executor(error.to_string()))?;
        let workspace_text = posix_text(&workspace);
        let mut action = CanonicalObject::new();
        action.insert(
            JsonText::new("arguments_digest"),
            CanonicalValue::text(&arguments_digest),
        );
        action.insert(
            JsonText::new("destructive"),
            CanonicalValue::Bool(tool.descriptor().destructive()),
        );
        action.insert(
            JsonText::new("effective_capabilities"),
            CanonicalValue::array(
                capabilities
                    .iter()
                    .map(CanonicalValue::text)
                    .collect::<Vec<_>>(),
            ),
        );
        action.insert(
            JsonText::new("envelope_digest"),
            CanonicalValue::text(&envelope_digest),
        );
        action.insert(
            JsonText::new("idempotent"),
            CanonicalValue::Bool(tool.descriptor().idempotent()),
        );
        action.insert(
            JsonText::new("input_schema_digest"),
            CanonicalValue::text(tool.input_schema_digest()),
        );
        action.insert(
            JsonText::new("output_schema_digest"),
            CanonicalValue::text(tool.output_schema_digest()),
        );
        action.insert(
            JsonText::new("side_effect_level"),
            CanonicalValue::text(tool.side_effect_level().as_str()),
        );
        action.insert(
            JsonText::new("tool_descriptor_digest"),
            CanonicalValue::text(tool.descriptor_digest()),
        );
        action.insert(
            JsonText::new("tool_id"),
            CanonicalValue::text(tool.tool_id()),
        );
        action.insert(
            JsonText::new("tool_version"),
            CanonicalValue::text(tool.version()),
        );
        action.insert(
            JsonText::new("workspace"),
            CanonicalValue::text(&workspace_text),
        );
        let action_digest = canonical_json_digest(&CanonicalValue::Object(action))
            .map_err(|error| ToolExecutorError::executor(error.to_string()))?;
        Ok(ApprovalBinding {
            envelope,
            arguments_digest,
            workspace: workspace_text,
            envelope_digest,
            state_digest,
            action_digest,
            effective_capabilities: capabilities,
        })
    }

    /// Materialize the workspace and re-check the arguments after approval.
    fn prepare_dispatch(
        &self,
        call: &ToolCall,
        tool: &RegisteredTool,
        binding: &ApprovalBinding,
    ) -> Result<(PathBuf, ToolRequestEnvelope), CallRejected> {
        let Some(envelope) = binding.envelope.clone() else {
            return Err(CallRejected {
                code: ToolErrorCode::InvalidArguments,
                message: "the call arguments could not be canonicalized into a request envelope"
                    .to_owned(),
            });
        };
        let workspace = match workspace_for(&self.run_directory, call.call_id()) {
            Ok(workspace) => workspace,
            Err(error) => {
                return Err(CallRejected {
                    code: ToolErrorCode::WorkspaceViolation,
                    message: summarize_workspace_error(&error),
                });
            }
        };
        let arguments = match call.arguments_json().to_value() {
            Ok(value) => value,
            Err(error) => {
                return Err(CallRejected {
                    code: ToolErrorCode::InvalidArguments,
                    message: format!("JsonEnvelopeError: {error}"),
                });
            }
        };
        let CanonicalValue::Object(ref arguments) = arguments else {
            return Err(CallRejected {
                code: ToolErrorCode::InvalidArguments,
                message: "JsonEnvelopeError: arguments must be a JSON object".to_owned(),
            });
        };
        if let Err(error) = tool.validate_arguments(&CanonicalValue::Object(arguments.clone())) {
            return Err(CallRejected {
                code: if error.is_schema() {
                    ToolErrorCode::InvalidArguments
                } else {
                    ToolErrorCode::WorkspaceViolation
                },
                message: if error.is_schema() {
                    error.to_string()
                } else {
                    summarize_error(&error, None)
                },
            });
        }
        if let Err(error) = canonical_json_dumps(
            &CanonicalValue::Object(arguments.clone()),
            self.profile.max_argument_bytes(),
        ) {
            return Err(CallRejected {
                code: if error.is_json_envelope() {
                    ToolErrorCode::InvalidArguments
                } else {
                    ToolErrorCode::WorkspaceViolation
                },
                message: summarize_error(&error, None),
            });
        }
        if posix_text(&workspace) != binding.workspace {
            return Err(CallRejected {
                code: ToolErrorCode::WorkspaceViolation,
                message: "the resolved workspace changed between authorization and dispatch"
                    .to_owned(),
            });
        }
        Ok((workspace, envelope))
    }

    /// Authorize a call against the registry, the profile and the policy.
    fn authorize(
        &self,
        call: &ToolCall,
        tool: &RegisteredTool,
        binding: &ApprovalBinding,
    ) -> Option<(ToolErrorCode, String)> {
        let requested: BTreeSet<String> = call
            .requested_capabilities()
            .iter()
            .map(|value| value.as_str().to_owned())
            .collect();
        let required: BTreeSet<String> = tool
            .required_capabilities()
            .iter()
            .map(|value| value.as_str().to_owned())
            .collect();
        let granted = self.granted_capabilities();
        let effective: BTreeSet<String> = requested.union(&required).cloned().collect();
        let missing: Vec<String> = effective.difference(&granted).cloned().collect();
        if !missing.is_empty() {
            return Some((
                ToolErrorCode::CapabilityDenied,
                format!("capabilities are not granted: {}", missing.join(", ")),
            ));
        }
        let undeclared: Vec<String> = requested.difference(&required).cloned().collect();
        if !undeclared.is_empty() {
            return Some((
                ToolErrorCode::CapabilityDenied,
                format!(
                    "call requests capabilities the descriptor does not declare: {}",
                    undeclared.join(", ")
                ),
            ));
        }
        if let Err(error) = self
            .check_capability_policy(&effective)
            .and_then(|()| self.check_side_effect_policy(tool.side_effect_level()))
        {
            let code = self.policy_error_code(&effective, tool.side_effect_level());
            return Some((code, error.message().to_owned()));
        }
        if call.side_effect_level() != tool.side_effect_level() {
            return Some((
                ToolErrorCode::SideEffectMismatch,
                "the declared call side effect does not match the registered descriptor".to_owned(),
            ));
        }
        if let Some(policy) = &self.policy {
            let request = match self.request_for(call, tool, binding) {
                Ok(request) => request,
                Err(error) => {
                    return Some((
                        ToolErrorCode::CapabilityForbidden,
                        error.message().to_owned(),
                    ));
                }
            };
            match policy.decide(&request, None) {
                Ok(decision) => {
                    if !decision.allowed() {
                        return Some((
                            policy_decision_code(&decision),
                            format!(
                                "policy decision {} for request {}",
                                decision.reason().as_str(),
                                decision.request_id().as_str().unwrap_or_default()
                            ),
                        ));
                    }
                }
                Err(error) => {
                    return Some((
                        ToolErrorCode::CapabilityForbidden,
                        error.message().to_owned(),
                    ));
                }
            }
        }
        None
    }

    /// The policy request of a call.
    fn request_for(
        &self,
        call: &ToolCall,
        tool: &RegisteredTool,
        binding: &ApprovalBinding,
    ) -> ExecutorResult<PolicyRequest> {
        PolicyRequest::from_parts(
            kala_contracts::Text::new(call.call_id()),
            kala_contracts::Text::new(&binding.state_digest),
            kala_contracts::Text::new(&binding.action_digest),
            binding
                .effective_capabilities
                .iter()
                .map(|value| kala_contracts::Text::new(value.as_str())),
            tool.side_effect_level(),
            tool.descriptor().destructive(),
            tool.side_effect_level() == SideEffectLevel::Egress,
            None,
            Some(if tool.side_effect_level() == SideEffectLevel::Egress {
                ApprovalKind::Egress
            } else {
                ApprovalKind::ToolClassChange
            }),
        )
        .map_err(|error| ToolExecutorError::executor(error.message().to_owned()))
    }

    /// Refuse a capability the profile does not allow.
    fn check_capability_policy(&self, capabilities: &BTreeSet<String>) -> ExecutorResult<()> {
        let names: BTreeSet<&str> = capabilities.iter().map(String::as_str).collect();
        if !self.profile.allow_privileged_capabilities() {
            let forbidden: Vec<&str> = names
                .iter()
                .filter(|name| FORBIDDEN_CAPABILITIES.contains(*name))
                .copied()
                .collect();
            if !forbidden.is_empty() {
                return Err(ToolExecutorError::authority(format!(
                    "capabilities are denied by default: {}",
                    forbidden.join(", ")
                )));
            }
        }
        let network: Vec<&str> = names
            .iter()
            .filter(|name| NETWORK_CAPABILITIES.contains(*name))
            .copied()
            .collect();
        if !network.is_empty() && !self.profile.allow_network() {
            return Err(ToolExecutorError::authority(format!(
                "network capabilities are denied by default: {}",
                network.join(", ")
            )));
        }
        let egress: Vec<&str> = names
            .iter()
            .filter(|name| EGRESS_CAPABILITIES.contains(*name))
            .copied()
            .collect();
        if !egress.is_empty() && !(self.profile.allow_egress() && self.profile.allow_network()) {
            return Err(ToolExecutorError::authority(format!(
                "egress capabilities are denied by default: {}",
                egress.join(", ")
            )));
        }
        Ok(())
    }

    /// Refuse a side effect the profile does not allow.
    fn check_side_effect_policy(&self, side_effect_level: SideEffectLevel) -> ExecutorResult<()> {
        if side_effect_level == SideEffectLevel::Network && !self.profile.allow_network() {
            return Err(ToolExecutorError::authority(
                "network side effects are denied by default",
            ));
        }
        if side_effect_level == SideEffectLevel::Egress
            && !(self.profile.allow_egress() && self.profile.allow_network())
        {
            return Err(ToolExecutorError::authority(
                "egress side effects are denied by default",
            ));
        }
        Ok(())
    }

    /// The error code a capability or side effect refusal is reported with.
    fn policy_error_code(
        &self,
        capabilities: &BTreeSet<String>,
        side_effect_level: SideEffectLevel,
    ) -> ToolErrorCode {
        let names: BTreeSet<&str> = capabilities.iter().map(String::as_str).collect();
        if side_effect_level == SideEffectLevel::Egress
            || names.iter().any(|name| EGRESS_CAPABILITIES.contains(name))
        {
            return ToolErrorCode::EgressDenied;
        }
        if side_effect_level == SideEffectLevel::Network
            || names.iter().any(|name| NETWORK_CAPABILITIES.contains(name))
        {
            return ToolErrorCode::NetworkDenied;
        }
        ToolErrorCode::CapabilityForbidden
    }

    /// Choose how a tool may run, or refuse it.
    ///
    /// `Auto` never runs tool code inside the kernel. A subprocess handler is
    /// always dispatched out of process. An in-kernel handler is only eligible
    /// for the explicit trusted mode and only when the registry marked it as a
    /// first-party pure evaluator.
    fn select_isolation(&self, tool: &RegisteredTool) -> Option<IsolationMode> {
        let handler = tool.handler();
        if handler.is_subprocess() {
            if self.isolation == IsolationMode::TrustedInProcessEvaluator {
                return None;
            }
            return Some(IsolationMode::Subprocess);
        }
        if self.isolation == IsolationMode::Subprocess {
            return None;
        }
        if self.isolation == IsolationMode::Auto {
            return None;
        }
        if tool.trusted_in_process() && tool.side_effect_level() == SideEffectLevel::None {
            return Some(IsolationMode::TrustedInProcessEvaluator);
        }
        None
    }

    /// Refuse a side effect that the configured profile cannot back.
    fn sandbox_unavailable(&self, tool: &RegisteredTool) -> Option<String> {
        let verdict = kala_sandbox::judge_sandbox(
            self.profile.isolation_profile(),
            self.profile.sandbox_probe(),
        );
        if verdict.allowed {
            return None;
        }
        let side_effecting = matches!(
            tool.side_effect_level(),
            SideEffectLevel::LocalWrite | SideEffectLevel::Network | SideEffectLevel::Egress
        );
        if side_effecting || tool.descriptor().destructive() {
            return Some(format!(
                "tool {} is refused: {}",
                tool.tool_id(),
                verdict.reason
            ));
        }
        None
    }

    /// The sandbox root of a dispatch.
    fn sandbox_root(&self, call: &ToolCall) -> ExecutorResult<PathBuf> {
        let sequence = self.dispatch_sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let component = crate::executor::paths::call_component(call.call_id())?;
        Ok(self
            .run_directory
            .join("sandbox")
            .join(format!("{component}-{sequence}"))
            .join("root"))
    }

    /// Dispatch a call and account for it.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_call(
        &self,
        call: &ToolCall,
        tool: &RegisteredTool,
        binding: &ApprovalBinding,
        envelope: &ToolRequestEnvelope,
        workspace: &Path,
        isolation: IsolationMode,
        options: &ExecutionOptions<'_>,
        started_at: Timestamp,
    ) -> ToolResult {
        let mut reservation: Option<BudgetReservation> = None;
        if let Some(ledger) = &self.ledger {
            let usage = match BudgetUsage::new()
                .with_tool_calls(1u64)
                .try_with_duration_seconds(envelope.deadline_seconds())
            {
                Ok(usage) => usage,
                Err(error) => {
                    return self.denied(
                        call,
                        ToolErrorCode::BudgetExhausted,
                        &error.to_string(),
                        started_at,
                    );
                }
            };
            match ledger.reserve(&usage) {
                Ok(reserved) => reservation = Some(reserved),
                Err(error) => {
                    // Every budget failure of a reservation is an exhaustion:
                    // the ledger refuses a charge it cannot afford.
                    return self.denied(
                        call,
                        ToolErrorCode::BudgetExhausted,
                        &error.to_string(),
                        started_at,
                    );
                }
            }
        }
        self.dispatch_reserved(
            call,
            tool,
            binding,
            envelope,
            workspace,
            reservation.as_ref(),
            isolation,
            options,
            started_at,
        )
    }

    /// Dispatch a call that holds a budget reservation.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_reserved(
        &self,
        call: &ToolCall,
        tool: &RegisteredTool,
        binding: &ApprovalBinding,
        envelope: &ToolRequestEnvelope,
        workspace: &Path,
        reservation: Option<&BudgetReservation>,
        isolation: IsolationMode,
        options: &ExecutionOptions<'_>,
        started_at: Timestamp,
    ) -> ToolResult {
        let dispatch_started = Instant::now();
        let mut sandbox_root: Option<PathBuf> = None;
        let outcome = if isolation == IsolationMode::Subprocess {
            match self.sandbox_root(call) {
                Ok(root) => {
                    sandbox_root = Some(root);
                    self.dispatch_subprocess(
                        tool,
                        envelope,
                        workspace,
                        sandbox_root.as_deref(),
                        options,
                    )
                }
                Err(error) => Err(error),
            }
        } else {
            self.dispatch_in_process(tool, envelope, options)
        };
        if let Some(root) = sandbox_root.as_ref() {
            discard_sandbox_root(root);
        }
        let elapsed = dispatch_started.elapsed().as_secs_f64();
        let (response, process_id, failure) = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                self.release(reservation);
                return self.failure(
                    call,
                    ToolRunStatus::Failed,
                    ToolErrorCode::InternalError,
                    &summarize_executor_error(&error, Some(&workspace.to_string_lossy())),
                    started_at,
                    0.0,
                );
            }
        };
        if let Some(failure) = failure {
            self.release(reservation);
            self.record(
                tool,
                call,
                binding,
                isolation,
                failure.status,
                Some(failure.code),
                process_id,
                0,
            );
            if failure.status == ToolRunStatus::Cancelled {
                return self.cancelled(call, &failure.message, started_at, elapsed);
            }
            return self.failure(
                call,
                failure.status,
                failure.code,
                &failure.message,
                started_at,
                elapsed,
            );
        }
        let Some(response) = response else {
            self.release(reservation);
            self.record(
                tool,
                call,
                binding,
                isolation,
                ToolRunStatus::Failed,
                Some(ToolErrorCode::InternalError),
                process_id,
                0,
            );
            return self.failure(
                call,
                ToolRunStatus::Failed,
                ToolErrorCode::InternalError,
                "tool dispatch produced no result envelope",
                started_at,
                elapsed,
            );
        };
        let verified = match self.verify_response(&response, tool, call, workspace, elapsed) {
            Ok(verified) => verified,
            Err(error) => {
                self.release(reservation);
                let (code, message) = match &error {
                    ToolProtocolError::Schema { .. } => {
                        (ToolErrorCode::InvalidResult, error.to_string())
                    }
                    ToolProtocolError::Workspace(_) => (
                        ToolErrorCode::ArtifactRejected,
                        summarize_error(&error, Some(&workspace.to_string_lossy())),
                    ),
                    _ => (
                        ToolErrorCode::InvalidResult,
                        summarize_error(&error, Some(&workspace.to_string_lossy())),
                    ),
                };
                self.record(
                    tool,
                    call,
                    binding,
                    isolation,
                    ToolRunStatus::Failed,
                    Some(code),
                    process_id,
                    0,
                );
                return self.failure(
                    call,
                    ToolRunStatus::Failed,
                    code,
                    &message,
                    started_at,
                    elapsed,
                );
            }
        };
        let artifact_bytes: u64 = verified
            .artifacts()
            .iter()
            .map(|artifact| artifact.size_bytes().to_u64().unwrap_or(0))
            .sum();
        let budget_failure = self.commit(reservation, elapsed, artifact_bytes);
        let error_code = normalize_error_code(verified.error_code());
        self.record(
            tool,
            call,
            binding,
            isolation,
            verified.status(),
            error_code,
            process_id,
            artifact_bytes,
        );
        if let Some(message) = budget_failure {
            return self.failure(
                call,
                ToolRunStatus::Failed,
                ToolErrorCode::BudgetExhausted,
                &message,
                started_at,
                elapsed,
            );
        }
        match verified.to_result(&started_at, &now_timestamp()) {
            Ok(result) => result,
            Err(error) => self.failure(
                call,
                ToolRunStatus::Failed,
                ToolErrorCode::InternalError,
                &summarize_error(&error, Some(&workspace.to_string_lossy())),
                started_at,
                elapsed,
            ),
        }
    }

    /// Verify the answer of a tool.
    pub fn verify_response(
        &self,
        response: &ToolResponseEnvelope,
        tool: &RegisteredTool,
        call: &ToolCall,
        workspace: &Path,
        elapsed: f64,
    ) -> Result<ToolResponseEnvelope, ToolProtocolError> {
        if response.call_id() != call.call_id() {
            return Err(ToolProtocolError::protocol(
                "the tool response call id does not match the dispatched call",
            ));
        }
        if response.tool_id() != call.tool_id()
            || response.tool_version() != call.tool_version().as_str().unwrap_or_default()
        {
            return Err(ToolProtocolError::protocol(
                "the tool response identity does not match the dispatched call",
            ));
        }
        if !matches!(
            response.status(),
            ToolRunStatus::Succeeded | ToolRunStatus::Failed
        ) {
            return Err(ToolProtocolError::protocol(
                "a tool may only report a succeeded or failed run",
            ));
        }
        if response.status() == ToolRunStatus::Failed {
            if !response.artifacts().is_empty() {
                return Err(ToolProtocolError::protocol(
                    "a failed tool run cannot report artifacts",
                ));
            }
            let code = normalize_tool_error_code(response.error_code())
                .unwrap_or_else(|| ToolErrorCode::InternalError.as_str().to_owned());
            return ToolResponseEnvelope::new(
                response.protocol_version(),
                response.call_id(),
                response.tool_id(),
                response.tool_version(),
                ToolRunStatus::Failed,
                None,
                Some(&code),
                response.error_message(),
                &[],
                response.duration_ms(),
            );
        }
        if response.error_code().is_some() {
            return Err(ToolProtocolError::protocol(
                "a successful tool run cannot report an error code",
            ));
        }
        let Some(result) = response.result() else {
            return Err(ToolProtocolError::protocol(
                "a successful tool run requires a result payload",
            ));
        };
        tool.validate_result(&CanonicalValue::Object(result.clone()))?;
        let artifacts = self.verify_artifacts(response.artifacts(), workspace)?;
        ToolResponseEnvelope::new(
            response.protocol_version(),
            response.call_id(),
            response.tool_id(),
            response.tool_version(),
            ToolRunStatus::Succeeded,
            Some(result),
            None,
            None,
            &artifacts,
            round_three(elapsed * 1_000.0),
        )
    }

    /// Verify the artifacts a tool reported.
    pub fn verify_artifacts(
        &self,
        artifacts: &[ArtifactRef],
        workspace: &Path,
    ) -> Result<Vec<ArtifactRef>, ToolProtocolError> {
        let sink = self
            .result_sink
            .as_ref()
            .map(|sink| move |path: &Path| sink(path));
        let verified = verify_artifacts(
            artifacts,
            workspace,
            self.profile.max_artifact_bytes(),
            sink.as_ref().map(|sink| sink as &dyn Fn(&Path)),
        )
        .map_err(|error| match error.protocol_error() {
            Some(protocol) => protocol.clone(),
            None => ToolProtocolError::workspace(error.message().to_owned()),
        })?;
        Ok(verified
            .into_iter()
            .map(|entry| entry.artifact)
            .collect::<Vec<_>>())
    }

    /// Commit the budget of a finished run.
    fn commit(
        &self,
        reservation: Option<&BudgetReservation>,
        elapsed: f64,
        artifact_bytes: u64,
    ) -> Option<String> {
        let (Some(reserved), Some(ledger)) = (reservation, self.ledger.as_ref()) else {
            return None;
        };
        let duration = elapsed.min(reserved.usage().duration_seconds());
        let usage = BudgetUsage::new()
            .with_tool_calls(1u64)
            .try_with_duration_seconds(duration)
            .ok()?
            .with_artifact_bytes(artifact_bytes);
        match ledger.commit_with(reserved, &usage) {
            Ok(_) => None,
            Err(error) => {
                self.release_if_held(reserved);
                Some(error.to_string())
            }
        }
    }

    /// Release a budget reservation.
    fn release(&self, reservation: Option<&BudgetReservation>) {
        let (Some(reserved), Some(ledger)) = (reservation, self.ledger.as_ref()) else {
            return;
        };
        let _ = ledger.release(reserved);
    }

    /// Release a budget reservation, ignoring an unknown one.
    fn release_if_held(&self, reservation: &BudgetReservation) {
        let Some(ledger) = self.ledger.as_ref() else {
            return;
        };
        let _ = ledger.release(reservation);
    }

    /// Record the provenance of a run.
    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        tool: &RegisteredTool,
        call: &ToolCall,
        binding: &ApprovalBinding,
        isolation: IsolationMode,
        status: ToolRunStatus,
        error_code: Option<ToolErrorCode>,
        process_id: Option<u32>,
        artifact_bytes: u64,
    ) {
        let report = ToolExecutionReport {
            call_id: call.call_id().to_owned(),
            tool_id: tool.tool_id().to_owned(),
            tool_version: tool.version().to_owned(),
            isolation,
            sandbox_profile: self.profile.profile_name().to_owned(),
            status,
            error_code: error_code.map(|code| code.as_str().to_owned()),
            process_id,
            granted_capabilities: binding.effective_capabilities.clone(),
            artifact_bytes,
            enforced_controls: self.profile.enforced_controls(),
            envelope_digest: binding.envelope_digest.clone(),
        };
        self.reports
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(report);
    }

    /// Refuse a call.
    fn denied(
        &self,
        call: &ToolCall,
        code: ToolErrorCode,
        message: &str,
        started_at: Timestamp,
    ) -> ToolResult {
        self.failure(call, failure_status(code), code, message, started_at, 0.0)
    }

    /// Report a cancelled call.
    fn cancelled(
        &self,
        call: &ToolCall,
        message: &str,
        started_at: Timestamp,
        duration_seconds: f64,
    ) -> ToolResult {
        let envelope = match ToolResponseEnvelope::failed_envelope(
            call.call_id(),
            call.tool_id(),
            call.tool_version().as_str().unwrap_or_default(),
            ToolRunStatus::Cancelled,
            None,
            Some(message),
            round_three(duration_seconds * 1_000.0),
        ) {
            Ok(envelope) => envelope,
            Err(_) => {
                return ToolResult::from_value(&CanonicalValue::Object(CanonicalObject::new()))
                    .unwrap_or_else(|_| unreachable!("the empty result is valid"));
            }
        };
        match envelope.to_result(&started_at, &now_timestamp()) {
            Ok(result) => result,
            Err(_) => ToolResult::from_value(&CanonicalValue::Object(CanonicalObject::new()))
                .unwrap_or_else(|_| unreachable!("the empty result is valid")),
        }
    }

    /// Report a failed call.
    fn failure(
        &self,
        call: &ToolCall,
        status: ToolRunStatus,
        code: ToolErrorCode,
        message: &str,
        started_at: Timestamp,
        duration_seconds: f64,
    ) -> ToolResult {
        let truncated: String = message.chars().take(MAX_ERROR_MESSAGE_LENGTH).collect();
        let message = if truncated.is_empty() {
            code.as_str().to_owned()
        } else {
            truncated
        };
        let envelope = match ToolResponseEnvelope::failed_envelope(
            call.call_id(),
            call.tool_id(),
            call.tool_version().as_str().unwrap_or_default(),
            status,
            Some(code.as_str()),
            Some(&message),
            round_three(duration_seconds * 1_000.0),
        ) {
            Ok(envelope) => envelope,
            Err(_) => {
                return ToolResult::from_value(&CanonicalValue::Object(CanonicalObject::new()))
                    .unwrap_or_else(|_| unreachable!("the empty result is valid"));
            }
        };
        match envelope.to_result(&started_at, &now_timestamp()) {
            Ok(result) => result,
            Err(_) => ToolResult::from_value(&CanonicalValue::Object(CanonicalObject::new()))
                .unwrap_or_else(|_| unreachable!("the empty result is valid")),
        }
    }

    /// The effective capabilities of a call.
    ///
    /// A call may only use a capability the executor granted and that the
    /// tool declared or the call requested.
    fn effective_capabilities(&self, call: &ToolCall, tool: &RegisteredTool) -> Vec<String> {
        let granted = self.granted_capabilities();
        let required: Vec<String> = tool
            .required_capabilities()
            .iter()
            .map(|value| value.as_str().to_owned())
            .collect();
        let requested: Vec<String> = call
            .requested_capabilities()
            .iter()
            .map(|value| value.as_str().to_owned())
            .collect();
        effective_capabilities(&granted, &required, &requested)
    }

    /// The import roots the child may read.
    fn child_import_roots(&self) -> Vec<PathBuf> {
        let mut candidates = self.import_roots.clone();
        candidates.push(package_root());
        candidates
    }

    /// Dispatch a call to a worker that runs out of process.
    fn dispatch_subprocess(
        &self,
        tool: &RegisteredTool,
        envelope: &ToolRequestEnvelope,
        workspace: &Path,
        sandbox_root: Option<&Path>,
        options: &ExecutionOptions<'_>,
    ) -> Result<DispatchOutcome, ToolExecutorError> {
        let dispatch = crate::executor::dispatch::SubprocessDispatch {
            python_executable: self.python_executable.as_deref(),
            import_roots: &self.child_import_roots(),
            extra_environment: self.profile.extra_environment(),
            sandbox_init: self.sandbox_init.as_deref(),
            interpreter_layout: &self.interpreter_layout,
            max_output_bytes: self.profile.max_output_bytes(),
            max_artifact_bytes: self.profile.max_artifact_bytes(),
            enforce_child_limits: self.profile.enforce_child_limits(),
            sandbox_root,
            strict: self.profile.isolation_profile() == IsolationProfile::StrictLinux,
            controls: &self.probe_controls,
        };
        crate::executor::dispatch::dispatch_subprocess(
            tool,
            envelope,
            workspace,
            &dispatch,
            options.cancel,
        )
    }

    /// Dispatch a call to a handler that runs inside the kernel process.
    fn dispatch_in_process(
        &self,
        tool: &RegisteredTool,
        envelope: &ToolRequestEnvelope,
        options: &ExecutionOptions<'_>,
    ) -> Result<DispatchOutcome, ToolExecutorError> {
        crate::executor::dispatch::dispatch_in_process(
            tool,
            envelope,
            self.isolation,
            options.cancel,
            self.profile.max_output_bytes(),
        )
    }
}

/// The error code a tool reported, normalized to a code the kernel knows.
fn normalize_error_code(value: Option<&str>) -> Option<ToolErrorCode> {
    let candidate = value?;
    match ToolErrorCode::parse(candidate) {
        Some(code) => Some(code),
        None => Some(ToolErrorCode::InternalError),
    }
}

/// The error code a tool reported, never a decision only the kernel may make.
fn normalize_tool_error_code(value: Option<&str>) -> Option<String> {
    let code = normalize_error_code(value);
    match code {
        None => None,
        Some(code) if is_kernel_decision(code) => {
            Some(ToolErrorCode::InternalError.as_str().to_owned())
        }
        Some(code) => Some(code.as_str().to_owned()),
    }
}

/// The error code a policy decision is reported with.
fn policy_decision_code(decision: &PolicyDecision) -> ToolErrorCode {
    match decision.reason() {
        PolicyReason::CapabilityNotGranted
        | PolicyReason::CapabilityExpired
        | PolicyReason::ScopeMismatch => ToolErrorCode::CapabilityDenied,
        PolicyReason::InvalidRequest => ToolErrorCode::CapabilityForbidden,
        // Every other reason needs an approval: the kernel decides, not the
        // tool.
        _ => ToolErrorCode::ApprovalRequired,
    }
}

/// The effective capabilities of a call.
fn effective_capabilities(
    granted: &BTreeSet<String>,
    required: &[String],
    requested: &[String],
) -> Vec<String> {
    let mut effective: BTreeSet<String> = BTreeSet::new();
    for name in required.iter().chain(requested.iter()) {
        if granted.contains(name) {
            effective.insert(name.clone());
        }
    }
    effective.into_iter().collect()
}

/// Normalize the granted capability names.
fn normalize_capabilities(
    values: impl Iterator<Item = impl AsRef<str>>,
) -> ExecutorResult<BTreeSet<String>> {
    let values: Vec<String> = values.map(|value| value.as_ref().to_owned()).collect();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for value in &values {
        let normalized = value.trim();
        if normalized.is_empty() || normalized.chars().count() > 128 {
            return Err(ToolExecutorError::executor(
                "granted capabilities must be nonempty and bounded",
            ));
        }
        if normalized
            .chars()
            .any(|character| character.is_whitespace() || (character as u32) < 32)
        {
            return Err(ToolExecutorError::executor(
                "granted capabilities contain unsafe characters",
            ));
        }
        names.insert(normalized.to_owned());
    }
    Ok(names)
}

/// Normalize the import roots of the child.
fn normalize_import_roots(roots: &[PathBuf]) -> ExecutorResult<Vec<PathBuf>> {
    let mut selected: Vec<PathBuf> = Vec::new();
    for root in roots {
        let candidate = kala_sandbox::resolve_strict_false(root);
        if !candidate.is_dir() {
            return Err(ToolExecutorError::executor(
                "import roots must be existing directories",
            ));
        }
        if !selected.contains(&candidate) {
            selected.push(candidate);
        }
    }
    if selected.is_empty() {
        selected.push(package_root());
    }
    Ok(selected)
}

/// The directory of the package that ships the kernel.
fn package_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Render a path the way `PurePath.as_posix` does.
fn posix_text(path: &Path) -> String {
    kala_sandbox::posix_text(path)
}

/// Round a duration to three decimals, like Python's `round`.
fn round_three(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

/// Render a float the way CPython's `str()` does.
///
/// The accepted `kala-json` crate owns the CPython compatible float
/// formatting, so a deadline is reported as `2.0` and not as `2`.
pub fn python_float(value: f64) -> String {
    kala_json::format_float(value).unwrap_or_else(|_| format!("{value}"))
}

/// The current UTC time.
fn now_timestamp() -> Timestamp {
    let now = kala_kernel::time::KernelTimestamp::now();
    let (year, month, day, hour, minute, second, microsecond) = now.utc_fields();
    Timestamp::new(year, month, day, hour, minute, second, microsecond, 0)
        .unwrap_or_else(|_| Timestamp::new(1970, 1, 1, 0, 0, 0, 0, 0).expect("the epoch"))
}

/// Summarize a workspace refusal.
fn summarize_workspace_error(error: &ToolExecutorError) -> String {
    match error.protocol_error() {
        Some(protocol) => summarize_error(protocol, None),
        None => error.message().to_owned(),
    }
}

/// Summarize a refusal of the executor, exactly like the Python
/// `summarize_error`.
///
/// The class name is reported first, then the message. The message is
/// stripped, the host workspace is redacted, the whitespace is collapsed and
/// the detail is bounded.
pub fn summarize_error_with_class(class: &str, detail: &str, replacement: Option<&str>) -> String {
    let detail = detail.trim();
    if detail.is_empty() {
        return class.to_owned();
    }
    let detail = match replacement {
        Some(replacement) => detail.replace(replacement, "<workspace>"),
        None => detail.to_owned(),
    };
    let detail = collapse_whitespace(&detail);
    let detail: String = detail.chars().take(512).collect();
    format!("{class}: {detail}")
}

/// Summarize a refusal of the executor.
pub fn summarize_executor_error(error: &ToolExecutorError, replacement: Option<&str>) -> String {
    summarize_error_with_class(error.python_class_name(), error.message(), replacement)
}

/// Collapse a run of whitespace into a single space.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(character);
            in_space = false;
        }
    }
    out
}

/// Remove the sandbox root of a dispatch.
fn discard_sandbox_root(root: &Path) {
    if let Some(parent) = root.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

/// The bounded worker pool of the executor.
struct WorkerSlots {
    capacity: usize,
    held: Mutex<usize>,
    released: Condvar,
}

impl WorkerSlots {
    /// Build a pool with the given capacity.
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            held: Mutex::new(0),
            released: Condvar::new(),
        }
    }

    /// Take a slot, waiting at most the queueing wait.
    fn acquire(&self, timeout_seconds: f64) -> bool {
        let timeout = Duration::from_secs_f64(timeout_seconds.max(0.0));
        let deadline = Instant::now() + timeout;
        let mut held = self.held.lock().unwrap_or_else(|error| error.into_inner());
        while *held >= self.capacity {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (guard, _) = self
                .released
                .wait_timeout(held, remaining)
                .unwrap_or_else(|error| error.into_inner());
            held = guard;
        }
        *held += 1;
        true
    }

    /// Give a slot back.
    fn release(&self) {
        let mut held = self.held.lock().unwrap_or_else(|error| error.into_inner());
        if *held > 0 {
            *held -= 1;
        }
        self.released.notify_one();
    }
}

/// The probe of the strict profile.
///
/// The portable profile is never probed: it confines a process, not a
/// filesystem.
fn probe_sandbox(
    profile: &SandboxProfile,
    import_roots: &[PathBuf],
    helper: Option<&Path>,
) -> SandboxProbe {
    if profile.isolation_profile() != IsolationProfile::StrictLinux {
        // The Python probe of the portable profile reports the platform as
        // "not-probed": the portable profile confines a process, not a
        // filesystem, so there is nothing to probe.
        return SandboxProbe {
            available: false,
            platform: "not-probed".to_owned(),
            controls: Vec::new(),
            detail: "the portable profile confines a process, not the filesystem".to_owned(),
            elapsed_ms: 0.0,
        };
    }
    let helper = match helper.map(Path::to_path_buf).or_else(discover_probe_helper) {
        Some(helper) => helper,
        None => {
            return SandboxProbe::unavailable(
                "the sandbox probe helper could not be found on this host",
                0.0,
            );
        }
    };
    let options = kala_sandbox::ProbeOptions {
        helper,
        import_roots: import_roots.to_vec(),
        scratch_bytes: profile.max_artifact_bytes().min(16 * 1_048_576) as i64,
        timeout: Duration::from_secs_f64(30.0),
    };
    kala_sandbox::probe_sandbox(&options)
}

/// The path of the native sandbox probe helper.
///
/// The helper is the accepted `kala-sandbox-probe` binary. It is looked up
/// next to the running program, because the crate that owns it is a sibling
/// workspace whose binaries are not installed next to the product.
fn discover_probe_helper() -> Option<PathBuf> {
    let candidates = ["kala-sandbox-probe", "kala-tools-sandbox-probe"];
    let mut directories: Vec<PathBuf> = Vec::new();
    if let Ok(current) = std::env::current_exe() {
        let mut directory = current.parent().map(Path::to_path_buf);
        if let Some(parent) = directory.as_ref().and_then(|value| value.parent()) {
            directory = Some(parent.to_path_buf());
        }
        if let Some(directory) = directory {
            directories.push(directory);
        }
    }
    for directory in directories {
        for candidate in candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// The classification of a header, re-exported for the tests.
pub use crate::executor::artifact::classify_header as classify_artifact_header;

/// The isolation profile of the executor.
pub use kala_sandbox::IsolationProfile as SandboxIsolationProfile;
