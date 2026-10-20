//! The supervised Python geometry worker (ADR-0010, ADR-0011).
//!
//! The Studio front door never re-implements geometry. It supervises one
//! Python geometry worker process, reads exactly one readiness line from its
//! stdout, and then talks to it over an authenticated loopback HTTP API.
//!
//! # The launch contract
//!
//! * The command comes from `KALA_STUDIO_GEOMETRY_WORKER`. The value is
//!   either a JSON argv array (`["/path/python", "-m", "kala_agent.studio",
//!   "--worker"]`) or a single executable path, in which case the fixed
//!   module arguments `-m kala_agent.studio --worker` are appended.
//!   Whitespace splitting is **not** performed: a value that names an
//!   executable plus arguments is refused rather than silently re-parsed.
//! * The development default is `<repo>/.venv/bin/python -m kala_agent.studio
//!   --worker`, resolved from the repository that contains this crate.
//! * The bearer token is 32 fresh random bytes rendered as 64 hexadecimal
//!   characters. It is passed **only** through `KALA_WORKER_TOKEN`, never in
//!   argv, and it is never logged.
//! * `KALA_WORKER_STDIN_LIFELINE=1` is set so the worker stops when this
//!   process closes its stdin: a dead front door can never orphan a worker.
//! * `KALA_STEP_IMPORT_MAX_BYTES` carries the **resolved** STEP import
//!   ceiling, so the worker and the front door accept exactly the same
//!   body. The value is resolved once, by the front door, with the worker's
//!   own parsing and bounds ([`crate::step_limit`]).
//! * `--data-dir <front-door data dir>` is appended to the worker argv. The
//!   worker refuses to start without an explicit data directory, so the
//!   front door's journal, artifacts and settings are the ones the worker
//!   writes; a front door and a Python reference server started on the same
//!   data directory share one journal, and parallel tests never share one.
//!
//! # The launch implementation
//!
//! The command, the working directory and the child environment are built
//! with the accepted `kala-process` crate (`ProcessSpec` and
//! `ChildEnvironment`), so the absolute-executable rule, the canonical
//! working directory and the environment validation are the reviewed ones.
//! `kala-process` supervises a process *to completion* and therefore has no
//! streaming or long-lived API; the persistent spawn, the readiness line and
//! the restart loop live here, in the crate that owns the worker.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use kala_process::{CancelToken, ChildEnvironment, ProcessSpec};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::step_limit::STEP_IMPORT_MAX_BYTES_ENV;

/// The environment variable that names the worker command.
pub const WORKER_COMMAND_ENV_VAR: &str = "KALA_STUDIO_GEOMETRY_WORKER";
/// The environment variable that carries the per-launch bearer token.
pub const WORKER_TOKEN_ENV_VAR: &str = "KALA_WORKER_TOKEN";
/// The environment variable that enables the stdin lifeline.
pub const WORKER_LIFELINE_ENV_VAR: &str = "KALA_WORKER_STDIN_LIFELINE";
/// The prefix of the single readiness line the worker prints.
pub const WORKER_READY_PREFIX: &str = "KALA_WORKER_READY";
/// The loopback host the worker binds.
pub const WORKER_HOST: &str = "127.0.0.1";
/// The number of random bytes in the per-launch bearer token.
pub const WORKER_TOKEN_BYTES: usize = 32;
/// The rendered length of the hexadecimal bearer token.
pub const WORKER_TOKEN_LENGTH: usize = WORKER_TOKEN_BYTES * 2;
/// The bounded time the front door waits for the readiness line.
pub const WORKER_START_TIMEOUT: Duration = Duration::from_secs(30);
/// The first restart delay after a worker exit.
pub const WORKER_RESTART_MIN_DELAY: Duration = Duration::from_millis(250);
/// The largest restart delay the backoff grows to.
pub const WORKER_RESTART_MAX_DELAY: Duration = Duration::from_secs(5);
/// The grace period the worker gets to exit after its stdin is closed.
pub const WORKER_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
/// The internal route prefix the worker mounts.
pub const WORKER_INTERNAL_PREFIX: &str = "/internal/v1";
/// The fixed arguments appended to a single-executable worker command.
pub const FIXED_WORKER_ARGUMENTS: &[&str] = &["-m", "kala_agent.studio", "--worker"];
/// The argv flag that carries the front door's resolved data directory.
pub const WORKER_DATA_DIR_FLAG: &str = "--data-dir";

/// Why a worker command could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerCommandError {
    /// The configured value is empty.
    Empty,
    /// The configured value is not a JSON argv array.
    NotAnArgvArray,
    /// The configured value is a JSON array that is empty or holds a
    /// non-string entry.
    InvalidArgvArray,
    /// The configured value names an executable and arguments; whitespace
    /// splitting is refused on purpose.
    WhitespaceNotAllowed,
    /// The configured executable is not an absolute path.
    NotAbsolute,
}

impl std::fmt::Display for WorkerCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the geometry worker command is empty"),
            Self::NotAnArgvArray => formatter.write_str(
                "the geometry worker command must be a JSON argv array or a single executable path",
            ),
            Self::InvalidArgvArray => formatter.write_str(
                "the geometry worker command array must hold at least one string entry",
            ),
            Self::WhitespaceNotAllowed => formatter.write_str(
                "the geometry worker command must not be split on whitespace; use a JSON argv array",
            ),
            Self::NotAbsolute => {
                formatter.write_str("the geometry worker executable must be an absolute path")
            }
        }
    }
}

impl std::error::Error for WorkerCommandError {}

/// One resolved worker launch command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerCommand {
    executable: PathBuf,
    arguments: Vec<String>,
}

impl WorkerCommand {
    /// Build a command from an explicit executable and arguments.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerCommandError::NotAbsolute`] for a relative executable.
    pub fn new(
        executable: impl Into<PathBuf>,
        arguments: impl IntoIterator<Item = String>,
    ) -> Result<Self, WorkerCommandError> {
        let executable = executable.into();
        if !executable.is_absolute() {
            return Err(WorkerCommandError::NotAbsolute);
        }
        Ok(Self {
            executable,
            arguments: arguments.into_iter().collect(),
        })
    }

    /// The explicit executable.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// The argv, excluding the executable.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    /// Parse the `KALA_STUDIO_GEOMETRY_WORKER` value.
    ///
    /// A JSON array is used as the argv verbatim. Any other value is one
    /// executable path and receives the fixed module arguments; a value that
    /// contains whitespace is refused rather than split.
    ///
    /// # Errors
    ///
    /// Returns the typed [`WorkerCommandError`] for an unusable value.
    pub fn parse(value: &str) -> Result<Self, WorkerCommandError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(WorkerCommandError::Empty);
        }
        if trimmed.starts_with('[') {
            let parsed: Value =
                serde_json::from_str(trimmed).map_err(|_| WorkerCommandError::NotAnArgvArray)?;
            let Some(entries) = parsed.as_array() else {
                return Err(WorkerCommandError::NotAnArgvArray);
            };
            if entries.is_empty() {
                return Err(WorkerCommandError::InvalidArgvArray);
            }
            let mut arguments = Vec::with_capacity(entries.len());
            for entry in entries {
                let Some(text) = entry.as_str() else {
                    return Err(WorkerCommandError::InvalidArgvArray);
                };
                if text.is_empty() {
                    return Err(WorkerCommandError::InvalidArgvArray);
                }
                arguments.push(text.to_owned());
            }
            let executable = PathBuf::from(&arguments[0]);
            if !executable.is_absolute() {
                return Err(WorkerCommandError::NotAbsolute);
            }
            return Ok(Self {
                executable,
                arguments: arguments.split_off(1),
            });
        }
        if trimmed.chars().any(char::is_whitespace) {
            return Err(WorkerCommandError::WhitespaceNotAllowed);
        }
        let executable = PathBuf::from(trimmed);
        if !executable.is_absolute() {
            return Err(WorkerCommandError::NotAbsolute);
        }
        Ok(Self {
            executable,
            arguments: FIXED_WORKER_ARGUMENTS
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect(),
        })
    }

    /// The development default: the repository virtualenv running the Studio
    /// worker module.
    #[must_use]
    pub fn development_default() -> Option<Self> {
        for root in candidate_repository_roots() {
            let interpreter = root.join(".venv").join("bin").join("python");
            if interpreter.is_file() {
                return Self::new(
                    interpreter,
                    FIXED_WORKER_ARGUMENTS
                        .iter()
                        .map(|argument| (*argument).to_owned()),
                )
                .ok();
            }
            let windows_interpreter = root.join(".venv").join("Scripts").join("python.exe");
            if windows_interpreter.is_file() {
                return Self::new(
                    windows_interpreter,
                    FIXED_WORKER_ARGUMENTS
                        .iter()
                        .map(|argument| (*argument).to_owned()),
                )
                .ok();
            }
        }
        None
    }

    /// The command resolved from the environment, or the development default.
    #[must_use]
    pub fn from_environment(environ: &BTreeMap<String, String>) -> Option<Self> {
        match environ.get(WORKER_COMMAND_ENV_VAR) {
            Some(value) if !value.trim().is_empty() => Self::parse(value).ok(),
            _ => Self::development_default(),
        }
    }
}

/// The repository roots the development default is searched in.
fn candidate_repository_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(explicit) = std::env::var("KALA_STUDIO_REPO_ROOT") {
        if !explicit.trim().is_empty() {
            roots.push(PathBuf::from(explicit));
        }
    }
    if let Ok(executable) = std::env::current_exe() {
        let mut cursor = executable.parent();
        while let Some(directory) = cursor {
            roots.push(directory.to_path_buf());
            cursor = directory.parent();
        }
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    roots
}

/// The import roots handed to the worker process.
#[must_use]
pub fn development_import_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for root in candidate_repository_roots() {
        let source = root.join("src");
        if source
            .join("kala_agent")
            .join("studio")
            .join("__init__.py")
            .is_file()
        {
            roots.push(source);
            break;
        }
    }
    roots
}

/// Mint a fresh worker bearer token.
///
/// The token is 32 random bytes rendered as 64 hexadecimal characters, which
/// satisfies the worker's `[A-Za-z0-9._~-]` grammar and its 32 character
/// minimum without any extra encoding.
///
/// # Errors
///
/// Returns a message when the operating system entropy source fails.
pub fn mint_worker_token() -> Result<String, String> {
    let mut bytes = [0u8; WORKER_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| format!("worker token entropy failed: {error}"))?;
    let mut token = String::with_capacity(WORKER_TOKEN_LENGTH);
    for byte in bytes {
        token.push_str(&format!("{byte:02x}"));
    }
    Ok(token)
}

/// The readiness facts the worker announces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerReadiness {
    /// The loopback port the worker bound.
    pub port: u16,
    /// The Studio version the worker reports.
    pub version: String,
    /// Whether the worker's OCP geometry kernel is importable.
    pub design_available: bool,
}

/// Parse the single readiness line the worker prints.
///
/// # Errors
///
/// Returns a message when the line does not carry the three documented
/// fields with the documented types.
pub fn parse_readiness_line(line: &str) -> Result<WorkerReadiness, String> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let Some(payload) = trimmed.strip_prefix(WORKER_READY_PREFIX) else {
        return Err("the worker did not print a readiness line".to_owned());
    };
    let payload = payload.trim_start();
    let parsed: Value = serde_json::from_str(payload)
        .map_err(|error| format!("the readiness line is not JSON: {error}"))?;
    let port = parsed
        .get("port")
        .and_then(Value::as_u64)
        .ok_or_else(|| "the readiness line has no port".to_owned())?;
    let port =
        u16::try_from(port).map_err(|_| "the readiness line has an invalid port".to_owned())?;
    let version = parsed
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| "the readiness line has no version".to_owned())?
        .to_owned();
    let design_available = parsed
        .get("design_available")
        .and_then(Value::as_bool)
        .ok_or_else(|| "the readiness line has no design_available flag".to_owned())?;
    Ok(WorkerReadiness {
        port,
        version,
        design_available,
    })
}

/// The lifecycle state the worker is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerState {
    /// The worker is being launched and has not announced readiness.
    Starting,
    /// The worker announced readiness and answers requests.
    Ready,
    /// The worker exited and a restart is pending.
    Restarting,
    /// The supervisor was shut down on purpose.
    Stopped,
    /// The worker could not be launched.
    Failed,
}

impl WorkerState {
    /// The stable wire name of the state.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Restarting => "restarting",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

/// The worker state the health route publishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerStatus {
    /// The lifecycle state.
    pub state: WorkerState,
    /// The Studio version the worker reported, when it is ready.
    pub version: Option<String>,
    /// Whether the worker's geometry kernel is importable.
    pub design_available: bool,
}

/// The live endpoint of a ready worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerEndpoint {
    /// The loopback port the worker bound.
    pub port: u16,
    /// The per-launch bearer token.
    pub token: Arc<str>,
}

/// The supervisor of one geometry worker.
pub struct GeometryWorker {
    command: WorkerCommand,
    data_dir: PathBuf,
    workspace: PathBuf,
    import_roots: Vec<PathBuf>,
    step_import_max_bytes: usize,
    status: RwLock<WorkerStatus>,
    endpoint: RwLock<Option<WorkerEndpoint>>,
    cancel: CancelToken,
    task: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}

impl GeometryWorker {
    /// Build a supervisor for one worker command.
    ///
    /// `data_dir` is the front door's resolved Studio data directory. It is
    /// handed to the worker as `--data-dir`, so the worker's journal and
    /// artifacts live exactly where the front door says they do; there is no
    /// home-directory or temporary-directory fallback in worker mode.
    ///
    /// `step_import_max_bytes` is the **resolved** STEP import ceiling. It
    /// is handed to the worker through `KALA_STEP_IMPORT_MAX_BYTES`, so the
    /// worker and the front door enforce the same bound.
    #[must_use]
    pub fn new(
        command: WorkerCommand,
        data_dir: PathBuf,
        workspace: PathBuf,
        import_roots: Vec<PathBuf>,
        step_import_max_bytes: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            command,
            data_dir,
            workspace,
            import_roots,
            step_import_max_bytes,
            status: RwLock::new(WorkerStatus {
                state: WorkerState::Stopped,
                version: None,
                design_available: false,
            }),
            endpoint: RwLock::new(None),
            cancel: CancelToken::new(),
            task: tokio::sync::Mutex::new(None),
        })
    }

    /// The command this supervisor launches.
    #[must_use]
    pub fn command(&self) -> &WorkerCommand {
        &self.command
    }

    /// The front door data directory handed to the worker as `--data-dir`.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The argv the worker process is launched with.
    ///
    /// The configured arguments come first and the front door's
    /// `--data-dir` pair is always last, so an explicit data directory in the
    /// configured argv cannot win over the front door's resolved one. The
    /// path is carried as an OS string, so a non-UTF-8 directory is not
    /// altered.
    #[must_use]
    pub fn launch_arguments(&self) -> Vec<std::ffi::OsString> {
        let mut arguments: Vec<std::ffi::OsString> = self
            .command
            .arguments()
            .iter()
            .map(std::ffi::OsString::from)
            .collect();
        arguments.push(std::ffi::OsString::from(WORKER_DATA_DIR_FLAG));
        arguments.push(self.data_dir.as_os_str().to_owned());
        arguments
    }

    /// The STEP import ceiling the worker process is launched with.
    #[must_use]
    pub fn step_import_max_bytes(&self) -> usize {
        self.step_import_max_bytes
    }

    /// The current worker status.
    pub async fn status(&self) -> WorkerStatus {
        self.status.read().await.clone()
    }

    /// The live endpoint, when the worker is ready.
    pub async fn endpoint(&self) -> Option<WorkerEndpoint> {
        self.endpoint.read().await.clone()
    }

    /// Start the supervision loop.
    pub async fn start(self: &Arc<Self>) {
        let worker = Arc::clone(self);
        let handle = tokio::spawn(async move { worker.supervise().await });
        let mut task = self.task.lock().await;
        *task = Some(handle);
    }

    /// Stop the worker and wait for it to exit.
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let handle = self.task.lock().await.take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
        self.endpoint.write().await.take();
    }

    /// The supervision loop: launch, serve, restart with a capped backoff.
    async fn supervise(self: Arc<Self>) {
        let mut delay = WORKER_RESTART_MIN_DELAY;
        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            self.publish(WorkerState::Starting, None, false).await;
            match self.launch().await {
                Ok((child, stdin, endpoint, readiness)) => {
                    *self.endpoint.write().await = Some(endpoint.clone());
                    self.publish(
                        WorkerState::Ready,
                        Some(readiness.version.clone()),
                        readiness.design_available,
                    )
                    .await;
                    delay = WORKER_RESTART_MIN_DELAY;
                    self.wait_for_exit(child, stdin).await;
                    self.endpoint.write().await.take();
                }
                Err(_reason) => {
                    self.publish(WorkerState::Failed, None, false).await;
                }
            }
            if self.cancel.is_cancelled() {
                break;
            }
            self.publish(WorkerState::Restarting, None, false).await;
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(WORKER_RESTART_MAX_DELAY);
        }
        self.publish(WorkerState::Stopped, None, false).await;
    }

    async fn publish(&self, state: WorkerState, version: Option<String>, design_available: bool) {
        let mut status = self.status.write().await;
        status.state = state;
        status.version = version;
        status.design_available = design_available;
    }

    /// Build the launch specification with the accepted process crate.
    fn process_spec(&self, token: &str) -> Result<ProcessSpec, String> {
        if !self.workspace.is_dir() {
            return Err("the geometry worker workspace is not a directory".to_owned());
        }
        let environment =
            ChildEnvironment::legacy_python_worker(&self.workspace, &self.import_roots)
                .map_err(|error| format!("the geometry worker environment is invalid: {error}"))?
                .with_extra(&[
                    (WORKER_TOKEN_ENV_VAR.to_owned(), token.to_owned()),
                    (WORKER_LIFELINE_ENV_VAR.to_owned(), "1".to_owned()),
                    (
                        STEP_IMPORT_MAX_BYTES_ENV.to_owned(),
                        self.step_import_max_bytes.to_string(),
                    ),
                ])
                .map_err(|error| format!("the geometry worker environment is invalid: {error}"))?;
        Ok(ProcessSpec::new(&self.command.executable, &self.workspace)
            .map_err(|error| format!("the geometry worker command is invalid: {error}"))?
            .args(self.launch_arguments())
            .with_environment(environment))
    }

    /// Launch the worker and read its readiness line.
    async fn launch(
        &self,
    ) -> Result<(Child, Option<ChildStdin>, WorkerEndpoint, WorkerReadiness), String> {
        let token = mint_worker_token()?;
        let spec = self.process_spec(&token)?;
        let mut command = Command::new(spec.executable());
        command.args(spec.arguments());
        command.current_dir(spec.working_directory());
        command.env_clear();
        for entry in spec.environment().entries() {
            command.env(&entry.key, &entry.value);
        }
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|error| format!("the geometry worker could not be launched: {error}"))?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // The worker logs to stderr; the front door never echoes a
                    // worker log line into an HTTP response.
                    let _ = line;
                }
            });
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "the geometry worker has no stdout".to_owned())?;
        let mut reader = BufReader::new(stdout);
        let deadline = tokio::time::Instant::now() + WORKER_START_TIMEOUT;
        loop {
            let mut line = String::new();
            let next = tokio::time::timeout_at(deadline, reader.read_line(&mut line)).await;
            match next {
                Ok(Ok(0)) => {
                    return Err(
                        "the geometry worker exited before it announced readiness".to_owned()
                    );
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) => {
                    return Err("the geometry worker closed its stdout unexpectedly".to_owned());
                }
                Err(_) => {
                    let _ = child.start_kill();
                    return Err("the geometry worker did not announce readiness in time".to_owned());
                }
            }
            let line = line.trim_end_matches(['\r', '\n']).to_owned();
            if line.trim().is_empty() {
                continue;
            }
            if !line.starts_with(WORKER_READY_PREFIX) {
                // The worker prints exactly one line; anything else on stdout
                // is not part of the contract and is ignored.
                continue;
            }
            let readiness = parse_readiness_line(&line)?;
            // Keep draining stdout: the worker redirects its own stdout to
            // stderr after the readiness line, but a closed pipe would turn
            // a late write into a broken pipe.
            tokio::spawn(async move {
                let mut lines = reader.lines();
                while let Ok(Some(_line)) = lines.next_line().await {}
            });
            let stdin = child.stdin.take();
            let endpoint = WorkerEndpoint {
                port: readiness.port,
                token: Arc::from(token.as_str()),
            };
            return Ok((child, stdin, endpoint, readiness));
        }
    }

    /// Wait for the worker to exit, or for the shutdown request.
    async fn wait_for_exit(&self, mut child: Child, stdin: Option<ChildStdin>) {
        let mut stdin = stdin;
        tokio::select! {
            result = child.wait() => {
                let _ = result;
                return;
            }
            () = self.cancelled() => {}
        }
        // Closing the stdin pipe is the documented lifeline: the worker stops
        // on EOF instead of outliving this process.
        drop(stdin.take());
        match tokio::time::timeout(WORKER_SHUTDOWN_GRACE, child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
    }

    async fn cancelled(&self) {
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_arrays_are_used_verbatim() {
        let command =
            WorkerCommand::parse(r#"["/usr/bin/python3","-m","kala_agent.studio","--worker"]"#)
                .unwrap();
        assert_eq!(command.executable(), Path::new("/usr/bin/python3"));
        assert_eq!(
            command.arguments(),
            [
                "-m".to_owned(),
                "kala_agent.studio".to_owned(),
                "--worker".to_owned()
            ]
        );
    }

    #[test]
    fn a_single_path_receives_the_fixed_arguments() {
        let command = WorkerCommand::parse("/usr/bin/python3").unwrap();
        assert_eq!(command.executable(), Path::new("/usr/bin/python3"));
        assert_eq!(
            command.arguments(),
            [
                "-m".to_owned(),
                "kala_agent.studio".to_owned(),
                "--worker".to_owned()
            ]
        );
    }

    #[test]
    fn whitespace_is_never_split() {
        assert_eq!(
            WorkerCommand::parse("/usr/bin/python3 -m kala_agent.studio --worker"),
            Err(WorkerCommandError::WhitespaceNotAllowed)
        );
    }

    #[test]
    fn relative_and_empty_commands_are_refused() {
        assert_eq!(WorkerCommand::parse("   "), Err(WorkerCommandError::Empty));
        assert_eq!(
            WorkerCommand::parse("python3"),
            Err(WorkerCommandError::NotAbsolute)
        );
        assert_eq!(
            WorkerCommand::parse("[]"),
            Err(WorkerCommandError::InvalidArgvArray)
        );
        assert_eq!(
            WorkerCommand::parse(r#"["python3"]"#),
            Err(WorkerCommandError::NotAbsolute)
        );
    }

    #[test]
    fn the_readiness_line_is_parsed() {
        let readiness = parse_readiness_line(
            "KALA_WORKER_READY {\"design_available\":true,\"port\":48525,\"version\":\"0.1.0\"}",
        )
        .unwrap();
        assert_eq!(readiness.port, 48525);
        assert_eq!(readiness.version, "0.1.0");
        assert!(readiness.design_available);
        assert!(parse_readiness_line("not ready").is_err());
        assert!(parse_readiness_line("KALA_WORKER_READY {\"port\":0}").is_err());
    }

    #[test]
    fn the_worker_token_is_fresh_hex_and_long_enough() {
        let first = mint_worker_token().unwrap();
        let second = mint_worker_token().unwrap();
        assert_eq!(first.len(), WORKER_TOKEN_LENGTH);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn the_child_environment_carries_the_resolved_step_import_limit() {
        let workspace = std::env::temp_dir().join("kala-studio-worker-env-test");
        std::fs::create_dir_all(&workspace).unwrap();
        let command = WorkerCommand::new("/usr/bin/python3", Vec::new()).unwrap();
        let worker = GeometryWorker::new(
            command,
            workspace.clone(),
            workspace.clone(),
            Vec::new(),
            crate::step_limit::DEFAULT_MAX_STEP_BYTES,
        );
        let spec = worker
            .process_spec("token-0123456789abcdef0123456789abcdef")
            .unwrap();
        let environment = spec.environment();
        assert_eq!(
            environment.get(STEP_IMPORT_MAX_BYTES_ENV),
            Some(
                crate::step_limit::DEFAULT_MAX_STEP_BYTES
                    .to_string()
                    .as_str()
            )
        );
        assert_eq!(
            environment.get(WORKER_TOKEN_ENV_VAR),
            Some("token-0123456789abcdef0123456789abcdef")
        );
        assert_eq!(environment.get(WORKER_LIFELINE_ENV_VAR), Some("1"));
        // The token is never an argument and the limit is not a secret.
        assert!(worker.command().arguments().is_empty());
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn the_front_door_data_dir_is_the_last_worker_argument() {
        let command = WorkerCommand::new(
            "/usr/bin/python3",
            vec!["-m".to_owned(), "kala_agent.studio".to_owned()],
        )
        .unwrap();
        let data_dir = std::env::temp_dir().join("kala-studio-front-door-data");
        let worker = GeometryWorker::new(
            command,
            data_dir.clone(),
            std::env::temp_dir(),
            Vec::new(),
            crate::step_limit::DEFAULT_MAX_STEP_BYTES,
        );
        let arguments = worker.launch_arguments();
        assert_eq!(
            &arguments[arguments.len() - 2..],
            [
                std::ffi::OsString::from(WORKER_DATA_DIR_FLAG),
                data_dir.as_os_str().to_owned()
            ]
        );
        // The configured argv stays untouched: the resolved pair is appended
        // last, so a stale `--data-dir` in KALA_STUDIO_GEOMETRY_WORKER loses.
        assert_eq!(worker.command().arguments().len(), 2);
        assert_eq!(worker.data_dir(), data_dir);
        assert!(
            arguments
                .iter()
                .all(|argument| !argument.to_string_lossy().contains("token"))
        );
    }
}
