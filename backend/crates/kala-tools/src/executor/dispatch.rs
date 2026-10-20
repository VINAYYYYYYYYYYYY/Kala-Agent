//! The dispatch of a tool call.
//!
//! A tool is untrusted. The dispatch starts the worker out of process, under
//! a deadline and a cancellation token, inside a bounded environment, and it
//! reads the answer through a bounded pipe. Every failure of the worker — a
//! crash, a hang, a flood, a malformed answer — is reported as a refusal
//! with a code the kernel understands, never as a panic and never as an
//! unsanitized diagnostic.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use kala_contracts::{SideEffectLevel, ToolRunStatus};
use kala_process::{
    CancelToken, ChildEnvironment, EnvironmentEntry, ProcessEnd, ProcessLimits, ProcessOutcome,
    ProcessSpec, supervise,
};
use kala_sandbox::{InterpreterLayout, SandboxSpec, WorkerCommand, WorkerJail};

use crate::executor::error::ToolExecutorError;
use crate::executor::paths::ensure_directory;
use crate::protocol::{
    ToolErrorCode, ToolRequestEnvelope, ToolResponseEnvelope, build_child_environment,
    build_worker_command, sanitize_child_stderr,
};
use crate::registry::RegisteredTool;

/// The failure of a dispatch.
#[derive(Clone, Debug)]
pub struct DispatchFailure {
    /// The status the caller sees.
    pub status: ToolRunStatus,
    /// The code the caller sees.
    pub code: ToolErrorCode,
    /// The message the caller sees.
    pub message: String,
}

/// The outcome of a dispatch: the answer, the worker pid and the failure.
pub type DispatchOutcome = (
    Option<ToolResponseEnvelope>,
    Option<u32>,
    Option<DispatchFailure>,
);

/// The polling interval of the in-process dispatch loop.
pub const POLL_INTERVAL_SECONDS: f64 = 0.02;

/// The size of the standard error sample.
pub const STDERR_SAMPLE_BYTES: usize = 65_536;

/// The time the executor waits for the worker to be reaped.
pub const WORKER_BOOTSTRAP_TIMEOUT_SECONDS: f64 = 5.0;

/// The exit code of a worker whose sandbox could not be established.
pub const ISOLATION_FAILURE_EXIT: i32 = 121;

/// The options of a subprocess dispatch.
pub struct SubprocessDispatch<'a> {
    /// The interpreter that runs the worker.
    pub python_executable: Option<&'a str>,
    /// The import roots the child may read.
    pub import_roots: &'a [PathBuf],
    /// The non-authority environment keys of the child.
    pub extra_environment: &'a BTreeMap<String, String>,
    /// The native init helper that builds the strict jail.
    pub sandbox_init: Option<&'a Path>,
    /// The interpreter layout the strict jail binds read-only.
    pub interpreter_layout: &'a InterpreterLayout,
    /// The output bound of the worker.
    pub max_output_bytes: usize,
    /// The artifact bound of the worker.
    pub max_artifact_bytes: u64,
    /// Whether the child resource limits are applied.
    pub enforce_child_limits: bool,
    /// The sandbox root of the dispatch, when the strict profile is used.
    pub sandbox_root: Option<&'a Path>,
    /// Whether the strict profile is configured.
    pub strict: bool,
    /// The controls the preflight probe verified.
    pub controls: &'a std::collections::BTreeSet<String>,
}

/// Dispatch a call to a worker that runs out of process.
pub fn dispatch_subprocess(
    tool: &RegisteredTool,
    envelope: &ToolRequestEnvelope,
    workspace: &Path,
    options: &SubprocessDispatch<'_>,
    cancel: Option<&CancelToken>,
) -> Result<DispatchOutcome, ToolExecutorError> {
    let handler = tool.handler().subprocess_handler().ok_or_else(|| {
        ToolExecutorError::isolation("the registered handler does not run out of process")
    })?;
    let mut sandbox_spec: Option<SandboxSpec> = None;
    if let Some(root) = options.sandbox_root {
        ensure_directory(root)?;
        sandbox_spec = build_sandbox_spec(options, root, workspace)?;
    }
    let python_executable = match options.python_executable {
        Some(value) if !value.is_empty() => value,
        _ => {
            if sandbox_spec.is_some() {
                return Err(ToolExecutorError::isolation(
                    "the strict profile needs an interpreter for the jailed worker",
                ));
            }
            return Err(ToolExecutorError::isolation(
                "no python interpreter is available for the tool worker",
            ));
        }
    };
    let command = build_worker_command(handler, Some(python_executable))
        .map_err(|error| ToolExecutorError::isolation(error.to_string()))?;
    let roots: Vec<&Path> = options.import_roots.iter().map(PathBuf::as_path).collect();
    let environment = build_child_environment(workspace, &roots, Some(options.extra_environment))
        .map_err(|error| ToolExecutorError::isolation(error.to_string()))?;
    let limits = ProcessLimits {
        wall_timeout: Duration::from_secs_f64(envelope.deadline_seconds().max(0.001)),
        bootstrap_timeout: Duration::from_secs_f64(WORKER_BOOTSTRAP_TIMEOUT_SECONDS),
        termination_timeout: Duration::from_secs_f64(WORKER_BOOTSTRAP_TIMEOUT_SECONDS),
        max_stdout_bytes: options.max_output_bytes.max(1),
        max_stderr_bytes: STDERR_SAMPLE_BYTES,
        max_stdin_bytes: crate::protocol::MAX_JSON_BYTES,
        enforce_resource_limits: options.enforce_child_limits,
        max_artifact_bytes: options.max_artifact_bytes.max(1),
    };
    let entries = environment
        .iter()
        .map(|(key, value)| EnvironmentEntry::new(key.clone(), value.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ToolExecutorError::isolation(error.to_string()))?;
    let child_environment = ChildEnvironment::from_entries(entries)
        .map_err(|error| ToolExecutorError::isolation(error.to_string()))?;
    let stdin = stdin_payload(envelope)?;
    let outcome = match sandbox_spec {
        Some(spec) => {
            let init = options.sandbox_init.ok_or_else(|| {
                ToolExecutorError::isolation(
                    "the strict profile needs the native sandbox init helper",
                )
            })?;
            let jail = WorkerJail {
                init: init.to_path_buf(),
                command: WorkerCommand::new(
                    PathBuf::from(command.first().cloned().unwrap_or_default()),
                    command
                        .iter()
                        .skip(1)
                        .map(std::ffi::OsString::from)
                        .collect(),
                ),
                spec,
                environment: Some(child_environment),
                limits: Some(limits),
            };
            jail.run(&stdin, &token(cancel))
                .map_err(|error| ToolExecutorError::executor(error.to_string()))?
        }
        None => {
            let executable = command.first().cloned().unwrap_or_default();
            let mut spec = ProcessSpec::new(executable, workspace)
                .map_err(|error| ToolExecutorError::isolation(error.to_string()))?;
            spec = spec.args(command.iter().skip(1).cloned());
            spec = spec.with_environment(child_environment);
            spec = spec.with_limits(limits);
            supervise(&spec, &stdin, &token(cancel))
                .map_err(|error| ToolExecutorError::executor(error.to_string()))?
        }
    };
    Ok(classify_outcome(&outcome, envelope, workspace))
}

/// Build the strict specification of a dispatch.
///
/// The portable profile builds no jail: it confines a process, not a
/// filesystem. The strict profile builds the jail the probe verified, so a
/// control the host could not establish is never requested.
fn build_sandbox_spec(
    options: &SubprocessDispatch<'_>,
    root: &Path,
    workspace: &Path,
) -> Result<Option<SandboxSpec>, ToolExecutorError> {
    if !options.strict {
        return Ok(None);
    }
    let spec = kala_sandbox::build_sandbox_spec_with(
        options.controls,
        root,
        workspace,
        options.import_roots,
        options.max_artifact_bytes.min(16 * 1_048_576) as i64,
        &kala_sandbox::child_resource_paths(options.interpreter_layout),
    )
    .map_err(|error| ToolExecutorError::isolation(error.to_string()))?;
    Ok(Some(spec))
}

/// The request bytes the worker reads from its standard input.
fn stdin_payload(envelope: &ToolRequestEnvelope) -> Result<Vec<u8>, ToolExecutorError> {
    let mut payload = envelope
        .to_json()
        .map_err(|error| ToolExecutorError::executor(error.to_string()))?;
    payload.push('\n');
    Ok(payload.into_bytes())
}

/// The cancellation token of a dispatch.
fn token(cancel: Option<&CancelToken>) -> CancelToken {
    cancel.cloned().unwrap_or_default()
}

/// Turn a supervised process into a dispatch outcome.
fn classify_outcome(
    outcome: &ProcessOutcome,
    envelope: &ToolRequestEnvelope,
    workspace: &Path,
) -> DispatchOutcome {
    let process_id = outcome.process_id;
    let stderr = sanitize_child_stderr(
        outcome.stderr.as_bytes(),
        Some(&workspace.to_string_lossy()),
    );
    match &outcome.end {
        ProcessEnd::Cancelled(_) => (
            None,
            process_id,
            Some(DispatchFailure {
                status: ToolRunStatus::Cancelled,
                code: ToolErrorCode::Cancelled,
                message: "the tool call was cancelled during execution".to_owned(),
            }),
        ),
        ProcessEnd::TimedOut(_) => (
            None,
            process_id,
            Some(DispatchFailure {
                status: ToolRunStatus::TimedOut,
                code: ToolErrorCode::Timeout,
                message: format!(
                    "tool exceeded the {} second deadline",
                    crate::executor::python_float(envelope.deadline_seconds())
                ),
            }),
        ),
        ProcessEnd::OutputLimitExceeded(_) => (
            None,
            process_id,
            Some(DispatchFailure {
                status: ToolRunStatus::Failed,
                code: ToolErrorCode::OutputTooLarge,
                message: "tool output exceeds the bounded output size".to_owned(),
            }),
        ),
        ProcessEnd::Exited(status) => {
            let exit_code = status.python_exit_code();
            if outcome.stdout.truncated() {
                return (
                    None,
                    process_id,
                    Some(DispatchFailure {
                        status: ToolRunStatus::Failed,
                        code: ToolErrorCode::OutputTooLarge,
                        message: "tool output exceeds the bounded output size".to_owned(),
                    }),
                );
            }
            if exit_code == i64::from(ISOLATION_FAILURE_EXIT) {
                return (
                    None,
                    process_id,
                    Some(DispatchFailure {
                        status: ToolRunStatus::Failed,
                        code: ToolErrorCode::SandboxUnavailable,
                        message: format!(
                            "the sandbox could not be established: {}",
                            if stderr.is_empty() {
                                "no detail"
                            } else {
                                &stderr
                            }
                        ),
                    }),
                );
            }
            if exit_code != 0 {
                return (
                    None,
                    process_id,
                    Some(DispatchFailure {
                        status: ToolRunStatus::Failed,
                        code: ToolErrorCode::Crashed,
                        message: format!(
                            "the tool process exited with status {exit_code}{}",
                            if stderr.is_empty() {
                                String::new()
                            } else {
                                format!(": {stderr}")
                            }
                        ),
                    }),
                );
            }
            match ToolResponseEnvelope::from_json(outcome.stdout.as_bytes()) {
                Ok(response) => (Some(response), process_id, None),
                Err(error) => (
                    None,
                    process_id,
                    Some(DispatchFailure {
                        status: ToolRunStatus::Failed,
                        code: ToolErrorCode::ProtocolError,
                        message: format!(
                            "the tool response envelope is invalid: {}{}",
                            error.python_class_name(),
                            if stderr.is_empty() {
                                String::new()
                            } else {
                                format!(": {stderr}")
                            }
                        ),
                    }),
                ),
            }
        }
    }
}

/// Dispatch a call to a handler that runs inside the kernel process.
///
/// The handler runs on its own thread so the deadline and the cancellation
/// are observed while it runs. A handler that is not a registered first-party
/// pure evaluator never reaches this path.
pub fn dispatch_in_process(
    tool: &RegisteredTool,
    envelope: &ToolRequestEnvelope,
    isolation: crate::executor::profile::IsolationMode,
    cancel: Option<&CancelToken>,
    max_output_bytes: usize,
) -> Result<DispatchOutcome, ToolExecutorError> {
    if tool.handler().is_subprocess() {
        return Err(ToolExecutorError::isolation(
            "subprocess handlers cannot run inside the kernel process",
        ));
    }
    if !tool.trusted_in_process() || tool.side_effect_level() != SideEffectLevel::None {
        return Err(ToolExecutorError::isolation(
            "only a registered first-party pure evaluator may run inside the kernel process",
        ));
    }
    if isolation != crate::executor::profile::IsolationMode::TrustedInProcessEvaluator {
        return Err(ToolExecutorError::isolation(
            "in-process execution requires the explicit trusted in-process evaluator mode",
        ));
    }
    let handler = tool.handler_handle();
    let request = envelope.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = handler.invoke(&request);
        let _ = sender.send(outcome);
    });
    let deadline = Instant::now() + Duration::from_secs_f64(envelope.deadline_seconds().max(0.001));
    loop {
        match receiver.recv_timeout(Duration::from_secs_f64(POLL_INTERVAL_SECONDS)) {
            Ok(Ok(response)) => {
                if let Some(cancel) = cancel {
                    if cancel.is_cancelled() {
                        return Ok((
                            None,
                            None,
                            Some(DispatchFailure {
                                status: ToolRunStatus::Cancelled,
                                code: ToolErrorCode::Cancelled,
                                message: "the tool call was cancelled during execution".to_owned(),
                            }),
                        ));
                    }
                }
                let payload_size = match response.to_json() {
                    Ok(payload) => payload.len(),
                    Err(_) => 0,
                };
                if payload_size > max_output_bytes {
                    return Ok((
                        None,
                        None,
                        Some(DispatchFailure {
                            status: ToolRunStatus::Failed,
                            code: ToolErrorCode::OutputTooLarge,
                            message: format!(
                                "result envelope of {payload_size} bytes exceeds the bounded output size"
                            ),
                        }),
                    ));
                }
                return Ok((Some(response), None, None));
            }
            Ok(Err(error)) => {
                return Ok((
                    None,
                    None,
                    Some(DispatchFailure {
                        status: ToolRunStatus::Failed,
                        code: ToolErrorCode::ProtocolError,
                        message: crate::executor::summarize_error_with_class(
                            error.python_class_name(),
                            &error.to_string(),
                            Some(envelope.workspace()),
                        ),
                    }),
                ));
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(cancel) = cancel {
                    if cancel.is_cancelled() {
                        return Ok((
                            None,
                            None,
                            Some(DispatchFailure {
                                status: ToolRunStatus::Cancelled,
                                code: ToolErrorCode::Cancelled,
                                message: "the tool call was cancelled during execution".to_owned(),
                            }),
                        ));
                    }
                }
                if Instant::now() >= deadline {
                    return Ok((
                        None,
                        None,
                        Some(DispatchFailure {
                            status: ToolRunStatus::TimedOut,
                            code: ToolErrorCode::Timeout,
                            message: format!(
                                "tool exceeded the {} second deadline",
                                crate::executor::python_float(envelope.deadline_seconds())
                            ),
                        }),
                    ));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Ok((
                    None,
                    None,
                    Some(DispatchFailure {
                        status: ToolRunStatus::Failed,
                        code: ToolErrorCode::InternalError,
                        message: "tool dispatch produced no result envelope".to_owned(),
                    }),
                ));
            }
        }
    }
}
