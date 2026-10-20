//! The sandbox profile and the isolation mode.
//!
//! The profile is the executor's configuration boundary: it fixes the output
//! bound, the argument bound, the artifact bound, the deadlines, the worker
//! concurrency cap and the authority the executor is allowed to hand to a
//! tool. The validation messages and their order are the Python ones, so a
//! refusal explains itself the same way in both implementations.

use std::collections::BTreeMap;

use kala_sandbox::{IsolationProfile, SandboxProbe};

use crate::executor::error::{ExecutorResult, ToolExecutorError};
use crate::protocol::PROTECTED_CHILD_ENVIRONMENT_KEYS;

/// The maximum number of concurrent tool workers.
pub const MAX_WORKERS: usize = 8;

/// The largest default deadline a profile may declare, in seconds.
pub const MAX_DEFAULT_TIMEOUT_SECONDS: f64 = 3_600.0;

/// The smallest deadline a profile may dispatch with, in seconds.
pub const MINIMUM_TIMEOUT_SECONDS: f64 = 0.01;

/// The maximum length of an error message the executor reports.
pub const MAX_ERROR_MESSAGE_LENGTH: usize = 1_024;

/// The largest queueing wait a profile may declare, in seconds.
pub const MAX_QUEUE_SECONDS: f64 = 300.0;

/// The smallest queueing wait a profile may declare, in seconds.
pub const MINIMUM_QUEUE_SECONDS: f64 = 0.0;

/// The version of the approval binding the executor writes.
pub const APPROVAL_BINDING_VERSION: &str = "tool-approval/2";

/// The environment keys a profile may add to the child environment.
///
/// The executor owns every key that bounds the child. A profile may only add
/// a key that carries no authority, so the list is closed.
pub const EXTRA_ENVIRONMENT_KEYS: [&str; 1] = ["TZ"];

/// The controls the executor always enforces.
///
/// The portable profile confines a process, not a filesystem. It never claims
/// a filesystem control.
pub const PORTABLE_CONTROLS: [&str; 10] = [
    "artifact_classification_kernel_verified",
    "artifact_digest_verification",
    "cwd_confined",
    "environment_minimal",
    "file_descriptor_close",
    "output_size_limit",
    "process_group_kill",
    "process_isolation",
    "wall_clock_timeout",
    "worker_concurrency_cap",
];

/// The capability names that mean the tool may reach the network.
pub const NETWORK_CAPABILITIES: [&str; 5] = [
    "egress",
    "network",
    "network.access",
    "network.egress",
    "network.internet",
];

/// The capability names that mean the tool may send data out.
pub const EGRESS_CAPABILITIES: [&str; 3] = ["egress", "export", "network.egress"];

/// The capability names a tool may never hold unless the profile allows it.
pub const FORBIDDEN_CAPABILITIES: [&str; 4] = [
    "package.install",
    "process.spawn",
    "shell.execute",
    "system.admin",
];

/// How the executor is allowed to run a tool.
///
/// `Auto` is the default and never runs tool code inside the kernel: an
/// in-kernel handler is refused rather than silently invoked. Running inside
/// the kernel is only possible through the deliberately loud
/// `TrustedInProcessEvaluator` mode, which the registry restricts to
/// registered first-party pure evaluators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IsolationMode {
    /// Never run tool code inside the kernel process.
    Auto,
    /// Always run the tool out of process.
    Subprocess,
    /// Run a registered first-party pure evaluator inside the kernel process.
    TrustedInProcessEvaluator,
}

impl IsolationMode {
    /// The Python `StrEnum` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Subprocess => "subprocess",
            Self::TrustedInProcessEvaluator => "trusted_in_process_evaluator",
        }
    }

    /// Parse an exact Python `StrEnum` value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "subprocess" => Some(Self::Subprocess),
            "trusted_in_process_evaluator" => Some(Self::TrustedInProcessEvaluator),
            _ => None,
        }
    }
}

impl std::fmt::Display for IsolationMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The configuration of one tool executor.
///
/// The defaults are the portable profile: a bounded output, a bounded
/// argument, a bounded artifact, a thirty second deadline, four workers and
/// a five second queueing wait. The network, the egress and the privileged
/// capabilities are denied until a profile opens them explicitly.
#[derive(Clone, Debug, PartialEq)]
pub struct SandboxProfile {
    /// The name the executor reports in its provenance records.
    profile_name: String,
    /// The maximum number of bytes the worker may write to standard output.
    max_output_bytes: usize,
    /// The maximum number of bytes the canonical arguments may take.
    max_argument_bytes: usize,
    /// The maximum number of bytes an artifact may take.
    max_artifact_bytes: u64,
    /// The deadline a call uses when it does not declare one.
    default_timeout_seconds: f64,
    /// The number of tool workers that may run at the same time.
    max_workers: usize,
    /// The longest a call may wait for a worker slot.
    max_queue_seconds: f64,
    /// Whether the profile allows the network capabilities.
    allow_network: bool,
    /// Whether the profile allows the egress capabilities.
    allow_egress: bool,
    /// Whether the profile allows the privileged capabilities.
    allow_privileged_capabilities: bool,
    /// Whether the child resource limits are applied.
    enforce_child_limits: bool,
    /// The isolation profile the executor runs the tool under.
    isolation_profile: IsolationProfile,
    /// The non-authority environment keys the executor adds to the child.
    extra_environment: BTreeMap<String, String>,
    /// The preflight probe of the strict profile.
    sandbox_probe: Option<SandboxProbe>,
}

impl Default for SandboxProfile {
    fn default() -> Self {
        Self {
            profile_name: "portable-minimum".to_owned(),
            max_output_bytes: 262_144,
            max_argument_bytes: crate::protocol::MAX_ARGUMENT_BYTES,
            max_artifact_bytes: 8_388_608,
            default_timeout_seconds: 30.0,
            max_workers: 4,
            max_queue_seconds: 5.0,
            allow_network: false,
            allow_egress: false,
            allow_privileged_capabilities: false,
            enforce_child_limits: true,
            isolation_profile: IsolationProfile::Portable,
            extra_environment: BTreeMap::new(),
            sandbox_probe: None,
        }
    }
}

impl SandboxProfile {
    /// The portable profile.
    pub fn new() -> Self {
        Self::default()
    }

    /// A copy with a different profile name.
    pub fn with_profile_name(mut self, name: impl Into<String>) -> Self {
        self.profile_name = name.into();
        self
    }

    /// A copy with a different output bound.
    pub fn with_max_output_bytes(mut self, value: usize) -> Self {
        self.max_output_bytes = value;
        self
    }

    /// A copy with a different argument bound.
    pub fn with_max_argument_bytes(mut self, value: usize) -> Self {
        self.max_argument_bytes = value;
        self
    }

    /// A copy with a different artifact bound.
    pub fn with_max_artifact_bytes(mut self, value: u64) -> Self {
        self.max_artifact_bytes = value;
        self
    }

    /// A copy with a different default deadline.
    pub fn with_default_timeout_seconds(mut self, value: f64) -> Self {
        self.default_timeout_seconds = value;
        self
    }

    /// A copy with a different worker cap.
    pub fn with_max_workers(mut self, value: usize) -> Self {
        self.max_workers = value;
        self
    }

    /// A copy with a different queueing wait.
    pub fn with_max_queue_seconds(mut self, value: f64) -> Self {
        self.max_queue_seconds = value;
        self
    }

    /// A copy that allows the network capabilities.
    pub fn with_allow_network(mut self, value: bool) -> Self {
        self.allow_network = value;
        self
    }

    /// A copy that allows the egress capabilities.
    pub fn with_allow_egress(mut self, value: bool) -> Self {
        self.allow_egress = value;
        self
    }

    /// A copy that allows the privileged capabilities.
    pub fn with_allow_privileged_capabilities(mut self, value: bool) -> Self {
        self.allow_privileged_capabilities = value;
        self
    }

    /// A copy that applies or drops the child resource limits.
    pub fn with_enforce_child_limits(mut self, value: bool) -> Self {
        self.enforce_child_limits = value;
        self
    }

    /// A copy with a different isolation profile.
    pub fn with_isolation_profile(mut self, value: IsolationProfile) -> Self {
        self.isolation_profile = value;
        self
    }

    /// A copy with a different set of non-authority environment keys.
    pub fn with_extra_environment(mut self, value: BTreeMap<String, String>) -> Self {
        self.extra_environment = value;
        self
    }

    /// A copy with a different preflight probe.
    pub fn with_sandbox_probe(mut self, value: Option<SandboxProbe>) -> Self {
        self.sandbox_probe = value;
        self
    }

    /// The profile name.
    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }

    /// The output bound.
    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }

    /// The argument bound.
    pub fn max_argument_bytes(&self) -> usize {
        self.max_argument_bytes
    }

    /// The artifact bound.
    pub fn max_artifact_bytes(&self) -> u64 {
        self.max_artifact_bytes
    }

    /// The default deadline.
    pub fn default_timeout_seconds(&self) -> f64 {
        self.default_timeout_seconds
    }

    /// The worker cap.
    pub fn max_workers(&self) -> usize {
        self.max_workers
    }

    /// The queueing wait.
    pub fn max_queue_seconds(&self) -> f64 {
        self.max_queue_seconds
    }

    /// Whether the network capabilities are allowed.
    pub fn allow_network(&self) -> bool {
        self.allow_network
    }

    /// Whether the egress capabilities are allowed.
    pub fn allow_egress(&self) -> bool {
        self.allow_egress
    }

    /// Whether the privileged capabilities are allowed.
    pub fn allow_privileged_capabilities(&self) -> bool {
        self.allow_privileged_capabilities
    }

    /// Whether the child resource limits are applied.
    pub fn enforce_child_limits(&self) -> bool {
        self.enforce_child_limits
    }

    /// The isolation profile.
    pub fn isolation_profile(&self) -> IsolationProfile {
        self.isolation_profile
    }

    /// The non-authority environment keys.
    pub fn extra_environment(&self) -> &BTreeMap<String, String> {
        &self.extra_environment
    }

    /// The preflight probe.
    pub fn sandbox_probe(&self) -> Option<&SandboxProbe> {
        self.sandbox_probe.as_ref()
    }

    /// Validate the profile, exactly like the Python `__post_init__`.
    ///
    /// The order of the checks is the Python order, so the first refusal is
    /// the same one.
    pub fn validated(self) -> ExecutorResult<Self> {
        if self.profile_name.trim().is_empty() {
            return Err(ToolExecutorError::executor(
                "profile_name must be a nonempty string",
            ));
        }
        if self.max_output_bytes < 1 {
            return Err(ToolExecutorError::executor(
                "max_output_bytes must be a positive integer",
            ));
        }
        if self.max_argument_bytes < 1 {
            return Err(ToolExecutorError::executor(
                "max_argument_bytes must be a positive integer",
            ));
        }
        if self.max_artifact_bytes < 1 {
            return Err(ToolExecutorError::executor(
                "max_artifact_bytes must be a positive integer",
            ));
        }
        if self.max_workers < 1 || self.max_workers > MAX_WORKERS {
            return Err(ToolExecutorError::executor(format!(
                "max_workers must be between 1 and {MAX_WORKERS}"
            )));
        }
        if !(MINIMUM_QUEUE_SECONDS..=MAX_QUEUE_SECONDS).contains(&self.max_queue_seconds) {
            return Err(ToolExecutorError::executor(
                "max_queue_seconds must be within the supported bounds",
            ));
        }
        if !(self.default_timeout_seconds > 0.0
            && self.default_timeout_seconds <= MAX_DEFAULT_TIMEOUT_SECONDS)
        {
            return Err(ToolExecutorError::executor(
                "default_timeout_seconds must be within the supported bounds",
            ));
        }
        for (key, value) in &self.extra_environment {
            if PROTECTED_CHILD_ENVIRONMENT_KEYS.contains(&key.as_str()) {
                return Err(ToolExecutorError::executor(format!(
                    "extra_environment cannot override the protected key {key}"
                )));
            }
            if !EXTRA_ENVIRONMENT_KEYS.contains(&key.as_str()) {
                return Err(ToolExecutorError::executor(format!(
                    "extra_environment only accepts non-authority keys: {}",
                    EXTRA_ENVIRONMENT_KEYS.join(", ")
                )));
            }
            if value.contains('\u{0}') {
                return Err(ToolExecutorError::executor(
                    "extra_environment must map strings to strings",
                ));
            }
        }
        Ok(self)
    }

    /// The controls this profile actually enforces on this host.
    ///
    /// Every entry is a control the executor implements unconditionally, or a
    /// control a preflight probe established and verified inside a real child
    /// process. A control that was not established is never reported, so the
    /// portable profile never claims filesystem confinement.
    pub fn enforced_controls(&self) -> Vec<String> {
        let mut controls: Vec<String> = PORTABLE_CONTROLS
            .iter()
            .map(|control| (*control).to_owned())
            .collect();
        if self.enforce_child_limits && resource_limits_available() {
            controls.push("resource_limits".to_owned());
        }
        if self.isolation_profile == IsolationProfile::StrictLinux {
            if let Some(probe) = &self.sandbox_probe {
                if probe.available {
                    controls.extend(probe.controls.iter().cloned());
                }
            }
        }
        controls.sort();
        controls.dedup();
        controls
    }

    /// The deadline of a call.
    ///
    /// The requested deadline and the descriptor deadline are both honoured,
    /// the smaller one wins, and the profile's default is the ceiling. The
    /// deadline never drops below the documented minimum.
    pub fn timeout_for(&self, requested: Option<f64>, descriptor_timeout: f64) -> f64 {
        let mut candidates: Vec<f64> = Vec::with_capacity(2);
        if let Some(value) = requested {
            if value > 0.0 {
                candidates.push(value);
            }
        }
        if descriptor_timeout > 0.0 {
            candidates.push(descriptor_timeout);
        }
        let selected = candidates
            .iter()
            .copied()
            .fold(None::<f64>, |smallest, value| {
                Some(match smallest {
                    None => value,
                    Some(current) => current.min(value),
                })
            })
            .unwrap_or(self.default_timeout_seconds);
        MINIMUM_TIMEOUT_SECONDS.max(selected.min(self.default_timeout_seconds))
    }
}

/// Whether the host can apply the POSIX resource limits.
///
/// The Python implementation asks the interpreter whether the `resource`
/// module exists, which is true on every POSIX host and false on Windows.
pub fn resource_limits_available() -> bool {
    cfg!(unix)
}
