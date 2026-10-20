//! Typed process outcomes and their mapping to the Python executor statuses.

use std::borrow::Cow;
use std::str::Utf8Error;
use std::time::Duration;

use crate::compatibility::{CRASHED, OUTPUT_TOO_LARGE, SANDBOX_UNAVAILABLE, TIMEOUT};
use crate::sandbox::ISOLATION_FAILURE_EXIT;

/// One captured output stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
    discarded_bytes: u64,
}

impl CapturedOutput {
    /// Create an empty capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// The retained bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The number of retained bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether nothing was retained.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Whether the cap was reached and later bytes were discarded.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Bytes discarded after the cap.
    pub fn discarded_bytes(&self) -> u64 {
        self.discarded_bytes
    }

    /// Interpret the retained bytes as strict UTF-8. stdout protocol payloads
    /// must pass this; invalid bytes are preserved for diagnostics rather than
    /// silently replaced.
    pub fn as_str(&self) -> Result<&str, Utf8Error> {
        std::str::from_utf8(&self.bytes)
    }

    /// Interpret the retained bytes as UTF-8 with replacement, suitable for
    /// diagnostics only.
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }

    pub(crate) fn push(&mut self, chunk: &[u8], limit: usize) {
        let remaining = limit.saturating_sub(self.bytes.len());
        if chunk.len() <= remaining {
            self.bytes.extend_from_slice(chunk);
            return;
        }
        self.bytes.extend_from_slice(&chunk[..remaining]);
        self.truncated = true;
        self.discarded_bytes = self
            .discarded_bytes
            .saturating_add((chunk.len() - remaining) as u64);
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn discard(&mut self, count: usize) {
        if count > 0 {
            self.truncated = true;
            self.discarded_bytes = self.discarded_bytes.saturating_add(count as u64);
        }
    }
}

/// A native exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    /// The conventional shell-style status: the exit code for a normal exit,
    /// or `128 + signal` for a Unix signal death. Use
    /// [`ExitStatus::shell_exit_code`] when that representation is wanted.
    /// Python executor compatibility uses [`ExitStatus::python_exit_code`],
    /// which reports `-signal` instead.
    pub code: i64,
    /// The terminating signal on Unix, if the process was signalled.
    pub signal: Option<i32>,
}

impl ExitStatus {
    /// A normal exit code.
    pub fn exited(code: i64) -> Self {
        Self { code, signal: None }
    }

    /// A signal death. `code` carries the conventional shell status
    /// `128 + signal`; [`Self::python_exit_code`] reports `-signal`.
    pub fn signalled(signal: i32) -> Self {
        Self {
            code: 128 + i64::from(signal),
            signal: Some(signal),
        }
    }

    /// The status Python's `Popen.returncode` would report.
    ///
    /// A Unix signal death is negative (`-9` for `SIGKILL`), matching
    /// `subprocess.Popen.returncode`; a normal exit keeps its exact code.
    /// The Python executor embeds this value in its `tool_crashed` message.
    pub fn python_exit_code(&self) -> i64 {
        match self.signal {
            Some(signal) => -i64::from(signal),
            None => self.code,
        }
    }

    /// The conventional shell-style status (`128 + signal` on a signal death).
    ///
    /// This is provided explicitly so callers do not confuse it with
    /// [`Self::python_exit_code`]; the process outcome and compatibility
    /// mapping use the Python view.
    pub fn shell_exit_code(&self) -> i64 {
        self.code
    }
}

/// Which deadline expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutStage {
    /// The exec handshake or stdin delivery did not complete.
    Bootstrap,
    /// The total wall-clock deadline expired while the worker was running.
    WallClock,
}

/// When cancellation was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelStage {
    /// The token was already set; nothing was launched.
    BeforeLaunch,
    /// The token was set during the exec handshake.
    Bootstrap,
    /// The token was set while the worker was running or being read.
    Running,
}

/// Which stream exceeded its hard capture limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Why supervision ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessEnd {
    /// The worker exited, normally or by signal.
    Exited(ExitStatus),
    /// A deadline expired; the tree was killed.
    TimedOut(TimeoutStage),
    /// The caller cancelled; the tree was killed.
    Cancelled(CancelStage),
    /// A hard output cap was exceeded; the tree was killed.
    OutputLimitExceeded(OutputStream),
}

/// What cleanup was actually achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeCleanup {
    /// The direct child was reaped.
    pub direct_reaped: bool,
    /// The platform tree boundary was killed/terminated.
    pub tree_terminated: bool,
    /// No descendant remained in the native containment boundary.
    pub tree_empty: bool,
    /// stdout and stderr reached EOF before the termination deadline.
    pub pipes_drained: bool,
    /// The termination deadline expired.
    pub termination_timed_out: bool,
}

impl TreeCleanup {
    /// Every cleanup obligation was met.
    pub fn is_complete(&self) -> bool {
        self.direct_reaped
            && self.tree_terminated
            && self.tree_empty
            && self.pipes_drained
            && !self.termination_timed_out
    }
}

/// The complete result of one supervised process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutcome {
    /// Why supervision ended.
    pub end: ProcessEnd,
    /// The exit status when one was observed.
    pub exit: Option<ExitStatus>,
    /// Retained stdout bytes.
    pub stdout: CapturedOutput,
    /// Retained stderr bytes.
    pub stderr: CapturedOutput,
    /// The sanitised stderr summary, using the exact Python rules (control
    /// characters removed, first/last line retained, host workspace redacted,
    /// at most 512 characters).
    pub stderr_summary: String,
    /// Total elapsed time from launch to outcome decision.
    pub elapsed: Duration,
    /// The direct child pid, when one was created.
    pub process_id: Option<u32>,
    /// Native tree and pipe cleanup evidence.
    pub cleanup: TreeCleanup,
}

impl ProcessOutcome {
    /// True only for a clean exit with complete cleanup.
    pub fn succeeded(&self) -> bool {
        matches!(self.end, ProcessEnd::Exited(ExitStatus { code: 0, .. }))
            && self.cleanup.is_complete()
    }

    /// Strict UTF-8 view of retained stdout.
    pub fn stdout_str(&self) -> Result<&str, Utf8Error> {
        self.stdout.as_str()
    }

    /// Sanitised stderr summary, matching
    /// `kala_agent.tools.protocol.sanitize_child_stderr`.
    pub fn stderr_summary(&self) -> &str {
        &self.stderr_summary
    }

    /// The compatibility record a future native tools executor can translate
    /// into `ToolRunStatus` / `ToolErrorCode` without duplicating policy.
    ///
    /// Exact mappings implemented here:
    ///
    /// | outcome | `ToolRunStatus` | `ToolErrorCode` |
    /// |---|---|---|
    /// | exit 0 + complete cleanup | `SUCCEEDED` | none |
    /// | wall/bootstrap deadline | `TIMED_OUT` | `timeout` |
    /// | cancellation | `CANCELLED` | none (Python uses no error code) |
    /// | stdout cap exceeded | `FAILED` | `output_too_large` |
    /// | exit 121 | `FAILED` | `sandbox_unavailable` |
    /// | any other non-zero exit | `FAILED` | `tool_crashed` |
    /// | exit 0 but incomplete cleanup | `FAILED` | `tool_crashed` |
    pub fn compatibility(&self) -> Compatibility {
        match self.end {
            ProcessEnd::Cancelled(stage) => Compatibility {
                status: CompatibilityStatus::Cancelled,
                error_code: None,
                message: match stage {
                    CancelStage::BeforeLaunch => {
                        "the tool call was cancelled before dispatch".to_owned()
                    }
                    CancelStage::Bootstrap | CancelStage::Running => {
                        "the tool call was cancelled during execution".to_owned()
                    }
                },
            },
            ProcessEnd::TimedOut(stage) => Compatibility {
                status: CompatibilityStatus::TimedOut,
                error_code: Some(TIMEOUT),
                message: match stage {
                    TimeoutStage::Bootstrap => {
                        "the tool process did not complete its bootstrap before the deadline"
                            .to_owned()
                    }
                    TimeoutStage::WallClock => "tool exceeded its wall-clock deadline".to_owned(),
                },
            },
            ProcessEnd::OutputLimitExceeded(stream) => Compatibility {
                status: CompatibilityStatus::Failed,
                error_code: Some(OUTPUT_TOO_LARGE),
                message: match stream {
                    OutputStream::Stdout => {
                        "tool output exceeds the bounded output size".to_owned()
                    }
                    OutputStream::Stderr => {
                        "tool stderr exceeds the bounded diagnostic size".to_owned()
                    }
                },
            },
            ProcessEnd::Exited(status) if status.code == i64::from(ISOLATION_FAILURE_EXIT) => {
                Compatibility {
                    status: CompatibilityStatus::Failed,
                    error_code: Some(SANDBOX_UNAVAILABLE),
                    message: format!(
                        "the sandbox could not be established: {}",
                        if self.stderr_summary.is_empty() {
                            "no detail"
                        } else {
                            self.stderr_summary.as_str()
                        }
                    ),
                }
            }
            ProcessEnd::Exited(status) if status.code == 0 => {
                if self.cleanup.is_complete() {
                    Compatibility {
                        status: CompatibilityStatus::Succeeded,
                        error_code: None,
                        message: String::new(),
                    }
                } else {
                    Compatibility {
                        status: CompatibilityStatus::Failed,
                        error_code: Some(CRASHED),
                        message: format!(
                            "the tool process exited successfully but its containment boundary \
                             could not be fully cleaned up: {}",
                            self.cleanup_detail()
                        ),
                    }
                }
            }
            ProcessEnd::Exited(status) => Compatibility {
                status: CompatibilityStatus::Failed,
                error_code: Some(CRASHED),
                message: format!(
                    "the tool process exited with status {}{}",
                    status.python_exit_code(),
                    if self.stderr_summary.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", self.stderr_summary)
                    }
                ),
            },
        }
    }

    fn cleanup_detail(&self) -> String {
        let mut parts = Vec::new();
        if !self.cleanup.direct_reaped {
            parts.push("direct child not reaped");
        }
        if !self.cleanup.tree_terminated {
            parts.push("tree not terminated");
        }
        if !self.cleanup.tree_empty {
            parts.push("containment boundary not empty");
        }
        if !self.cleanup.pipes_drained {
            parts.push("pipes still held");
        }
        if self.cleanup.termination_timed_out {
            parts.push("termination deadline expired");
        }
        if parts.is_empty() {
            "unknown cleanup failure".to_owned()
        } else {
            parts.join(", ")
        }
    }
}

/// Python `ToolRunStatus` values that can result from dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityStatus {
    /// `ToolRunStatus.SUCCEEDED`.
    Succeeded,
    /// `ToolRunStatus.FAILED`.
    Failed,
    /// `ToolRunStatus.DENIED` (used only by callers mapping setup failures).
    Denied,
    /// `ToolRunStatus.TIMED_OUT`.
    TimedOut,
    /// `ToolRunStatus.CANCELLED`.
    Cancelled,
}

/// A status/error-code pair matching the Python executor's result fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compatibility {
    /// `ToolRunStatus`.
    pub status: CompatibilityStatus,
    /// `ToolErrorCode` string, or `None` for cancellation/success.
    pub error_code: Option<&'static str>,
    /// The `ToolResult.error_message` equivalent.
    pub message: String,
}
