//! Native bounded process supervision for the Kala Agent backend migration.
//!
//! This crate is a standalone, product-quality Rust replacement for the
//! portable parts of `kala_agent.tools.executor` and the strict Linux sandbox
//! in `kala_agent.tools.sandbox`. It deliberately stops at the process/sandbox
//! boundary: request envelopes, JSON schemas, policy, budgets, tool registries,
//! HTTP, worker pools, artifact classification and Solid Edge are *not* here.
//!
//! # Guarantees and honest boundaries
//!
//! * An explicit executable plus argv is executed directly. There is no shell,
//!   no implicit `sh -c`, no PATH search and no argument re-parsing.
//! * The child receives only the environment the caller built. Inherited
//!   secrets are never forwarded implicitly.
//! * The working directory is an absolute, canonicalised directory that the
//!   caller may resolve inside a run workspace with [`resolve_workspace_path`].
//!   That resolution confines a *path string*; it is not a filesystem jail.
//! * stdin, stdout and stderr are drained without deadlocking. stdout and
//!   stderr are bounded; stdout is exposed as strict UTF-8, stderr is reduced
//!   with the same sanitisation rules as the Python executor.
//! * Wall-clock, bootstrap and termination deadlines are enforced. The
//!   bootstrap deadline covers both the exec handshake and delivery/closure of
//!   stdin. Cancellation is observed before launch, during launch, while
//!   writing stdin and while waiting for exit. I/O is byte-budgeted so a
//!   continuously ready producer or reader cannot starve those decisions.
//! * The direct child is always reaped and the native containment boundary is
//!   closed on every outcome. Unix uses a dedicated process group and
//!   `SIGKILL`; Windows uses a Job Object with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, a suspended launch and an explicit
//!   handle-inheritance list.
//! * The strict Linux profile is advertised only after a throwaway native child
//!   has actually created and verified the namespace/mount/chroot jail with
//!   [`probe_sandbox`]. There is no Python sidecar and no Python product path.
//!
//! Runtime proof for the Windows path is pending: this crate was cross-checked
//! for `x86_64-pc-windows-msvc` on a Linux host but no Windows execution is
//! claimed. The Unix process-group boundary kills the worker's process group;
//! a descendant that deliberately creates a new session/group leaves that
//! boundary, which is reported rather than hidden. Windows Job Objects provide
//! the true child-tree boundary.
//!
//! # Python bootstrap transition
//!
//! The Python executor starts an interpreter with a `-c` bootstrap that reads a
//! JSON request from stdin and writes a JSON response. This crate does not
//! execute Python at product time. A future native tools executor supplies the
//! native worker executable/argv and the request bytes directly; the
//! [`build_child_environment`] helper is retained only as an exact
//! compatibility port for Python workers during the transition, and the
//! production constructor [`ChildEnvironment::minimal`] does not add any
//! interpreter variables.
//!
//! Exit-status compatibility deliberately follows `Popen.returncode`: a Unix
//! signal death reports `-signal` through [`ExitStatus::python_exit_code`],
//! while [`ExitStatus::shell_exit_code`] exposes the conventional `128+signal`
//! form separately.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod environment;
mod error;
mod limits;
mod outcome;
mod process;
mod sandbox;
#[cfg(target_os = "linux")]
mod sandbox_plan;
mod workspace;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_acl;

#[cfg(windows)]
pub use windows::{current_handle_count as windows_handle_count, current_process_in_job};
#[cfg(windows)]
pub use windows_acl::restrict_to_current_user as restrict_file_to_current_user;

pub use environment::{
    CHILD_ENVIRONMENT_KEYS, ChildEnvironment, EnvironmentEntry, PROCESS_PROTOCOL_VERSION,
    PROTECTED_CHILD_ENVIRONMENT_KEYS, TOOL_PROTOCOL_VERSION, build_child_environment, is_protected,
};
pub use error::{
    EnvironmentError, ProcessError, SandboxProbeError, SandboxSetupError, WorkspaceError,
};
pub use limits::{
    DEFAULT_BOOTSTRAP_TIMEOUT, DEFAULT_MAX_ARTIFACT_BYTES, DEFAULT_MAX_STDERR_BYTES,
    DEFAULT_MAX_STDIN_BYTES, DEFAULT_MAX_STDOUT_BYTES, DEFAULT_TERMINATION_TIMEOUT,
    DEFAULT_WALL_TIMEOUT, MAX_STDERR_BYTES, MAX_STDIN_BYTES, MAX_STDOUT_BYTES,
    MAX_TERMINATION_TIMEOUT, MAX_WALL_TIMEOUT, ProcessLimits,
};
pub use outcome::{
    CancelStage, CapturedOutput, Compatibility, CompatibilityStatus, ExitStatus, OutputStream,
    ProcessEnd, ProcessOutcome, TimeoutStage, TreeCleanup,
};
pub use process::{CancelToken, ProcessSpec, supervise};
pub use sandbox::{
    CLONE_NEWIPC, CLONE_NEWNET, CLONE_NEWNS, CLONE_NEWPID, CLONE_NEWUSER, CLONE_NEWUTS,
    ISOLATION_FAILURE_EXIT, IsolationProfile, MS_BIND, MS_NODEV, MS_NOEXEC, MS_NOSUID, MS_PRIVATE,
    MS_RDONLY, MS_REC, MS_REMOUNT, PROBE_FAILURE_EXIT, PROBE_TIMEOUT_SECONDS,
    STRICT_OPTIONAL_CONTROLS, STRICT_REQUIRED_CONTROLS, SandboxProbe, SandboxProbeOptions,
    SandboxSpec, SandboxVerdict, build_sandbox_spec, canary_is_invisible, device_list_is_minimal,
    devices_are_bound, enforced_controls, isolate_process, judge_sandbox, network_is_isolated,
    platform_name, portable_controls, probe_sandbox, procfs_is_namespaced, resolve_system_bindings,
    run_probe, sanitize_child_stderr, scratch_is_writable, system_bindings_are_readonly,
    unshadowed_scratch_paths, verify_controls, workspace_is_writable,
};
pub use workspace::{canonical_directory, resolve_workspace_directory, resolve_workspace_path};

/// Compatibility mapping constants used by the future native tools executor.
pub mod compatibility {
    /// Python `ToolErrorCode.TIMEOUT`.
    pub const TIMEOUT: &str = "timeout";
    /// Python `ToolErrorCode.CANCELLED` is represented by no error code on a
    /// cancelled `ToolResult`; this constant exists only for callers that need
    /// an internal label.
    pub const CANCELLED: &str = "cancelled";
    /// Python `ToolErrorCode.OUTPUT_TOO_LARGE`.
    pub const OUTPUT_TOO_LARGE: &str = "output_too_large";
    /// Python `ToolErrorCode.CRASHED` (`tool_crashed`).
    pub const CRASHED: &str = "tool_crashed";
    /// Python `ToolErrorCode.SANDBOX_UNAVAILABLE`.
    pub const SANDBOX_UNAVAILABLE: &str = "sandbox_unavailable";
    /// Python `ToolErrorCode.ISOLATION_UNAVAILABLE`.
    pub const ISOLATION_UNAVAILABLE: &str = "isolation_unavailable";
    /// Python `ToolErrorCode.INTERNAL_ERROR` (`tool_error`).
    pub const INTERNAL_ERROR: &str = "tool_error";
}
