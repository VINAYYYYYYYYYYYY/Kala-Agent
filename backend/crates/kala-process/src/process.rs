//! Explicit process specifications, cancellation and the supervisor entry
//! point.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::environment::ChildEnvironment;
use crate::error::ProcessError;
use crate::limits::ProcessLimits;
use crate::outcome::{CancelStage, CapturedOutput, ProcessEnd, ProcessOutcome, TreeCleanup};
use crate::sandbox::SandboxSpec;
use crate::workspace::canonical_directory;

/// A cooperative cancellation token.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    /// Create an unset token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Safe to call from any thread.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// An explicit executable, argv, working directory, environment and bounds.
///
/// There is no shell interpretation: [`ProcessSpec::arg`] appends exactly one
/// argv entry. The executable is always an absolute file path; PATH lookup is
/// never used.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub(crate) executable: PathBuf,
    pub(crate) arguments: Vec<OsString>,
    pub(crate) working_directory: PathBuf,
    pub(crate) environment: ChildEnvironment,
    pub(crate) limits: ProcessLimits,
    pub(crate) sandbox: Option<SandboxSpec>,
}

impl ProcessSpec {
    /// Start a specification for an absolute executable in an existing
    /// working directory.
    pub fn new(
        executable: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Result<Self, ProcessError> {
        let executable = executable.into();
        let working_directory = working_directory.into();
        if executable.as_os_str().is_empty() {
            return Err(ProcessError::InvalidSpec(
                "executable must not be empty".to_owned(),
            ));
        }
        if !executable.is_absolute() {
            return Err(ProcessError::InvalidSpec(
                "executable must be an absolute path; PATH lookup is never used".to_owned(),
            ));
        }
        if !executable.is_file() {
            return Err(ProcessError::spawn(
                &executable,
                "the executable is not a regular file",
            ));
        }
        let directory = canonical_directory(&working_directory).map_err(ProcessError::Workspace)?;
        let environment = ChildEnvironment::minimal(&directory)?;
        Ok(Self {
            executable,
            arguments: Vec::new(),
            working_directory: directory,
            environment,
            limits: ProcessLimits::default(),
            sandbox: None,
        })
    }

    /// Append exactly one argv entry. No shell expansion or splitting occurs.
    pub fn arg(mut self, argument: impl Into<OsString>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    /// Append argv entries in order.
    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    /// Replace the child environment.
    pub fn with_environment(mut self, environment: ChildEnvironment) -> Self {
        self.environment = environment;
        self
    }

    /// Replace all supervision limits.
    pub fn with_limits(mut self, limits: ProcessLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Request the strict Linux jail for this process.
    ///
    /// The working directory must be the sandbox workspace, and the
    /// specification must contain every required control. The Unix launcher
    /// creates the jail after `fork` and before `execve`; if any step fails the
    /// worker is never executed.
    pub fn with_sandbox(mut self, sandbox: SandboxSpec) -> Result<Self, ProcessError> {
        sandbox
            .validate_strict()
            .map_err(ProcessError::SandboxSetup)?;
        let sandbox_workspace =
            canonical_directory(&sandbox.workspace).map_err(ProcessError::Workspace)?;
        if sandbox_workspace != self.working_directory {
            return Err(ProcessError::InvalidSpec(
                "the sandbox workspace must equal the process working directory".to_owned(),
            ));
        }
        self.sandbox = Some(sandbox);
        Ok(self)
    }

    /// The explicit executable.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// The argv, excluding the executable.
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// The canonical working directory.
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    /// The explicit child environment.
    pub fn environment(&self) -> &ChildEnvironment {
        &self.environment
    }

    /// The configured bounds.
    pub fn limits(&self) -> &ProcessLimits {
        &self.limits
    }

    /// The strict sandbox specification, if one was requested.
    pub fn sandbox(&self) -> Option<&SandboxSpec> {
        self.sandbox.as_ref()
    }
}

/// Run one process to completion under the supplied bounds and token.
///
/// The returned outcome covers normal exits, signal deaths, cancellations,
/// deadline expiry and output-cap failures. Setup, launch, containment and
/// reaping failures are returned as [`ProcessError`]; cleanup has already been
/// attempted on every error path.
pub fn supervise(
    spec: &ProcessSpec,
    stdin: &[u8],
    cancel: &CancelToken,
) -> Result<ProcessOutcome, ProcessError> {
    spec.limits.validate().map_err(ProcessError::InvalidSpec)?;
    if stdin.len() > spec.limits.max_stdin_bytes {
        return Err(ProcessError::InvalidSpec(format!(
            "stdin payload of {} bytes exceeds the bounded input size of {} bytes",
            stdin.len(),
            spec.limits.max_stdin_bytes
        )));
    }
    spec.environment.validate()?;
    if !spec.executable.is_absolute() {
        return Err(ProcessError::InvalidSpec(
            "executable must be an absolute path".to_owned(),
        ));
    }
    if !spec.executable.is_file() {
        return Err(ProcessError::spawn(
            &spec.executable,
            "the executable is not a regular file",
        ));
    }
    let working = canonical_directory(&spec.working_directory).map_err(ProcessError::Workspace)?;
    if working != spec.working_directory {
        return Err(ProcessError::InvalidSpec(
            "the working directory changed after the specification was built".to_owned(),
        ));
    }
    if let Some(sandbox) = &spec.sandbox {
        sandbox
            .validate_strict()
            .map_err(ProcessError::SandboxSetup)?;
        let workspace = canonical_directory(&sandbox.workspace).map_err(ProcessError::Workspace)?;
        if workspace != spec.working_directory {
            return Err(ProcessError::InvalidSpec(
                "the sandbox workspace must equal the process working directory".to_owned(),
            ));
        }
    }
    if cancel.is_cancelled() {
        return Ok(cancelled_before_launch());
    }
    #[cfg(unix)]
    {
        crate::unix::supervise(spec, stdin, cancel)
    }
    #[cfg(windows)]
    {
        crate::windows::supervise(spec, stdin, cancel)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (spec, stdin, cancel);
        Err(ProcessError::Internal(
            "this target has no native process supervision backend".to_owned(),
        ))
    }
}

fn cancelled_before_launch() -> ProcessOutcome {
    ProcessOutcome {
        end: ProcessEnd::Cancelled(CancelStage::BeforeLaunch),
        exit: None,
        stdout: CapturedOutput::new(),
        stderr: CapturedOutput::new(),
        stderr_summary: String::new(),
        elapsed: std::time::Duration::ZERO,
        process_id: None,
        cleanup: TreeCleanup {
            direct_reaped: true,
            tree_terminated: true,
            tree_empty: true,
            pipes_drained: true,
            termination_timed_out: false,
        },
    }
}
