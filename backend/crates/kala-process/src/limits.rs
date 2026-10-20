//! Bounded supervision limits.
//!
//! The numeric defaults mirror the Python tool executor's portable profile:
//! a 30 second default deadline, a 262,144 byte stdout cap, a 65,536 byte
//! stderr sample, an 8 MiB artifact/file-size cap and a 5 second reaping
//! window. The bootstrap deadline is new in the native layer: it bounds the
//! spawn-to-exec handshake and the delivery of stdin before the worker runs.

use std::time::Duration;

/// Default total wall-clock deadline, matching `SandboxProfile`.
pub const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Default spawn-to-exec handshake and stdin delivery deadline.
pub const DEFAULT_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(5);
/// Default time allowed to kill, drain and reap after the worker ends.
pub const DEFAULT_TERMINATION_TIMEOUT: Duration = Duration::from_secs(5);
/// Default bounded stdout capture, matching `max_output_bytes`.
pub const DEFAULT_MAX_STDOUT_BYTES: usize = 262_144;
/// Default stderr sample, matching `_STDERR_SAMPLE_BYTES`.
pub const DEFAULT_MAX_STDERR_BYTES: usize = 65_536;
/// Default stdin payload bound, matching `MAX_JSON_BYTES` in the tool protocol.
pub const DEFAULT_MAX_STDIN_BYTES: usize = 1_048_576;
/// Hard upper bound accepted for a stdin payload.
pub const MAX_STDIN_BYTES: usize = 64 * 1_048_576;
/// Default `RLIMIT_FSIZE` value, matching `max_artifact_bytes`.
pub const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 8_388_608;

/// Hard upper bound accepted for a wall-clock deadline.
pub const MAX_WALL_TIMEOUT: Duration = Duration::from_secs(3_600);
/// Hard upper bound accepted for bootstrap and termination deadlines.
pub const MAX_TERMINATION_TIMEOUT: Duration = Duration::from_secs(300);
/// Hard upper bound accepted for a stdout capture.
pub const MAX_STDOUT_BYTES: usize = 64 * 1_048_576;
/// Hard upper bound accepted for a stderr sample.
pub const MAX_STDERR_BYTES: usize = 16 * 1_048_576;

/// All bounds applied to one supervised process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessLimits {
    /// Total time from fork/CreateProcess until the outcome is decided.
    pub wall_timeout: Duration,
    /// Time from launch until the exec handshake completes and stdin is
    /// closed. Expiry produces [`crate::TimeoutStage::Bootstrap`].
    pub bootstrap_timeout: Duration,
    /// Time allowed to kill the tree, drain pipes and reap after the outcome
    /// is decided.
    pub termination_timeout: Duration,
    /// Maximum stdout bytes retained.
    pub max_stdout_bytes: usize,
    /// Maximum stderr bytes retained. Reaching this cap truncates the sample;
    /// it is not itself a stdout-style failure, matching the Python executor.
    pub max_stderr_bytes: usize,
    /// Maximum accepted stdin payload. The caller is refused before launch if
    /// the payload is larger.
    pub max_stdin_bytes: usize,
    /// Apply POSIX resource limits in the child (`RLIMIT_CORE`,
    /// `RLIMIT_CPU`, `RLIMIT_FSIZE`, `RLIMIT_NOFILE`) and a `0077` umask.
    /// Windows has no equivalent and does not advertise this control.
    pub enforce_resource_limits: bool,
    /// Value used for `RLIMIT_FSIZE` when resource limits are enabled.
    pub max_artifact_bytes: u64,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            wall_timeout: DEFAULT_WALL_TIMEOUT,
            bootstrap_timeout: DEFAULT_BOOTSTRAP_TIMEOUT,
            termination_timeout: DEFAULT_TERMINATION_TIMEOUT,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            max_stderr_bytes: DEFAULT_MAX_STDERR_BYTES,
            max_stdin_bytes: DEFAULT_MAX_STDIN_BYTES,
            enforce_resource_limits: true,
            max_artifact_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
        }
    }
}

impl ProcessLimits {
    /// The Python-compatible defaults.
    pub fn python_defaults() -> Self {
        Self::default()
    }

    /// Validate every bound before anything is launched.
    pub fn validate(&self) -> Result<(), String> {
        if self.wall_timeout.is_zero() || self.wall_timeout > MAX_WALL_TIMEOUT {
            return Err(format!(
                "wall_timeout must be greater than zero and at most {} seconds",
                MAX_WALL_TIMEOUT.as_secs()
            ));
        }
        if self.bootstrap_timeout.is_zero() || self.bootstrap_timeout > MAX_WALL_TIMEOUT {
            return Err(format!(
                "bootstrap_timeout must be greater than zero and at most {} seconds",
                MAX_WALL_TIMEOUT.as_secs()
            ));
        }
        if self.termination_timeout.is_zero() || self.termination_timeout > MAX_TERMINATION_TIMEOUT
        {
            return Err(format!(
                "termination_timeout must be greater than zero and at most {} seconds",
                MAX_TERMINATION_TIMEOUT.as_secs()
            ));
        }
        if self.max_stdout_bytes == 0 || self.max_stdout_bytes > MAX_STDOUT_BYTES {
            return Err(format!(
                "max_stdout_bytes must be between 1 and {MAX_STDOUT_BYTES}"
            ));
        }
        if self.max_stderr_bytes == 0 || self.max_stderr_bytes > MAX_STDERR_BYTES {
            return Err(format!(
                "max_stderr_bytes must be between 1 and {MAX_STDERR_BYTES}"
            ));
        }
        if self.max_stdin_bytes == 0 || self.max_stdin_bytes > MAX_STDIN_BYTES {
            return Err(format!(
                "max_stdin_bytes must be between 1 and {MAX_STDIN_BYTES}"
            ));
        }
        if self.max_artifact_bytes == 0 {
            return Err("max_artifact_bytes must be positive".to_owned());
        }
        Ok(())
    }

    /// A copy with a different wall deadline.
    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self {
        self.wall_timeout = timeout;
        self
    }

    /// A copy with a different bootstrap deadline.
    pub fn with_bootstrap_timeout(mut self, timeout: Duration) -> Self {
        self.bootstrap_timeout = timeout;
        self
    }

    /// A copy with a different post-decision termination window.
    pub fn with_termination_timeout(mut self, timeout: Duration) -> Self {
        self.termination_timeout = timeout;
        self
    }

    /// A copy with different capture limits.
    pub fn with_output_limits(mut self, stdout: usize, stderr: usize) -> Self {
        self.max_stdout_bytes = stdout;
        self.max_stderr_bytes = stderr;
        self
    }
}
