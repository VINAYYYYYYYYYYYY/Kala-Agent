//! The shared machinery of the three subscription-CLI chat providers.
//!
//! A subscription provider does not hold a credential in Kala. It runs the
//! operator's own, already-authenticated command-line tool as a subprocess and
//! lets that tool talk to its vendor. Everything in this module exists to
//! make that subprocess boring:
//!
//! * **No shell.** The child is started from an argv array with
//!   [`tokio::process::Command`]. Nothing is ever handed to a shell, so no
//!   prompt text, model id or path can be interpreted as shell syntax.
//! * **The prompt never rides on argv.** The prompt is written to the child's
//!   standard input and the standard input is closed afterwards. A prompt is
//!   therefore invisible in `ps` output and cannot overflow the argument
//!   limit.
//! * **A sanitized environment.** The child starts from an empty environment
//!   (`env_clear`) and receives only the names on [`ENV_ALLOWLIST`] plus the
//!   two names the CLI itself needs to find its own login. Nothing of Kala's
//!   configuration - no stored key, no studio token, no data directory - is
//!   inherited, and no `*_API_KEY` is forwarded: the CLI reads its own
//!   credential store, Kala never sees it.
//! * **A private working directory.** Every run gets a fresh temporary
//!   directory that is deleted when the turn ends, so the CLI cannot read or
//!   write the project the Studio happens to run in.
//! * **A bounded wall time.** The turn is bounded by
//!   [`DEFAULT_CLI_TIMEOUT_SECONDS`] (configurable through
//!   [`CLI_TIMEOUT_ENV`]), and a child that overruns is killed and reaped.
//! * **Bounded output.** Standard output is capped at
//!   [`MAX_CLI_STDOUT_BYTES`] and standard error at [`MAX_CLI_STDERR_BYTES`].
//!
//! The availability probe is deliberately cheap and never reads a credential
//! file: it asks the CLI for its version and, when the CLI has one, for its
//! own authentication status. Only the exit status of that status command is
//! used; its output is never echoed, so an account name or an address can
//! never leak into a response.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use super::ProviderFailure;

/// The default wall-clock budget of one CLI chat turn.
pub const DEFAULT_CLI_TIMEOUT_SECONDS: u64 = 180;
/// The shortest wall-clock budget an operator may configure.
pub const MIN_CLI_TIMEOUT_SECONDS: u64 = 1;
/// The longest wall-clock budget an operator may configure.
pub const MAX_CLI_TIMEOUT_SECONDS: u64 = 3_600;
/// The environment variable that configures the wall-clock budget.
pub const CLI_TIMEOUT_ENV: &str = "KALA_STUDIO_CLI_TIMEOUT_SECONDS";
/// The largest standard output one CLI turn may produce.
pub const MAX_CLI_STDOUT_BYTES: usize = 1_000_000;
/// The largest standard error one CLI turn may produce.
pub const MAX_CLI_STDERR_BYTES: usize = 65_536;
/// The wall-clock budget of one availability probe.
///
/// The probe is a background refresh, not a request-bound operation: a CLI
/// that hangs on `--version` must never hold a route open. The cap is short
/// on purpose, because the verdict only fills a provider row.
pub const CLI_PROBE_TIMEOUT_SECONDS: u64 = 2;
/// How long a probe verdict is reused before it is taken again.
pub const CLI_PROBE_TTL_SECONDS: u64 = 120;
/// How long `GET /providers`, `GET /settings` and `GET /health` wait for a
/// first probe verdict before they answer with the honest "checking" row.
///
/// The route budget plus the response work stays below one second, so the
/// Studio boot gate never fails because a CLI is slow.
pub const PROBE_ROUTE_BUDGET_MILLIS: u64 = 600;
/// The pause between two looks at the probe cache while a route waits.
pub const PROBE_POLL_MILLIS: u64 = 10;
/// The honest reason of a row whose first probe has not finished yet.
pub const PROBE_CHECKING_REASON: &str = "checking...";
/// The longest excerpt of a CLI failure that is reported to the operator.
pub const MAX_CLI_MESSAGE_LENGTH: usize = 512;
/// How many times a start that the kernel refused as "text file busy" is
/// retried.
pub const MAX_BUSY_START_ATTEMPTS: u32 = 4;
/// The pause between two such attempts.
pub const BUSY_START_BACKOFF_MILLIS: u64 = 50;

/// The environment names every subscription CLI receives.
///
/// The list is the union of "where the CLI finds its own login" (`HOME`, the
/// `XDG_*` directories), "how the CLI finds its own programs" (`PATH`) and
/// the locale and temporary-directory names. It deliberately carries no
/// credential, no Kala variable and no Studio token.
pub const ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
];

/// The identity and the fixed probe commands of one subscription CLI.
#[derive(Clone, Copy, Debug)]
pub struct CliSpec {
    /// The provider id this CLI answers for.
    pub id: &'static str,
    /// The name the provider list shows.
    pub name: &'static str,
    /// The executable name looked up on `PATH`.
    pub binary: &'static str,
    /// The environment variable that overrides the executable path.
    pub path_override_env: &'static str,
    /// The cheap version probe.
    pub version_args: &'static [&'static str],
    /// The cheap authentication-status probe.
    pub auth_args: &'static [&'static str],
    /// The extra environment names this CLI needs to find its own login.
    pub env_passthrough: &'static [&'static str],
}

/// The verdict of the availability probe of one subscription CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliProbe {
    /// The provider id this verdict belongs to.
    pub id: &'static str,
    /// Whether the CLI is installed and reports a version.
    pub installed: bool,
    /// Whether the CLI reports that it is signed in.
    pub authenticated: bool,
    /// Whether the first probe is still running and no verdict exists yet.
    ///
    /// A checking row is honest about not knowing: it reports
    /// `available: null` and [`PROBE_CHECKING_REASON`], never "unavailable".
    pub checking: bool,
    /// The honest reason, shown next to the provider row.
    pub reason: String,
}

impl CliProbe {
    /// Whether the CLI can answer a turn right now.
    #[must_use]
    pub fn usable(&self) -> bool {
        self.installed && self.authenticated
    }

    /// The honest placeholder for a CLI whose first probe is still running.
    #[must_use]
    pub fn checking(id: &'static str) -> Self {
        Self {
            id,
            installed: false,
            authenticated: false,
            checking: true,
            reason: PROBE_CHECKING_REASON.to_owned(),
        }
    }
}

/// The result of one finished CLI run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliRun {
    /// The exit code, when the child was terminated by a normal exit.
    pub status: Option<i32>,
    /// The bounded standard output.
    pub stdout: Vec<u8>,
    /// The bounded standard error.
    pub stderr: Vec<u8>,
}

impl CliRun {
    /// Whether the child exited with status zero.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.status == Some(0)
    }
}

/// The wall-clock budget of one CLI turn, read from the environment.
///
/// An unreadable or out-of-range value falls back to
/// [`DEFAULT_CLI_TIMEOUT_SECONDS`] rather than failing the turn: a bad
/// configuration must not make the provider unavailable.
#[must_use]
pub fn timeout_seconds(environ: &BTreeMap<String, String>) -> u64 {
    let Some(raw) = environ.get(CLI_TIMEOUT_ENV) else {
        return DEFAULT_CLI_TIMEOUT_SECONDS;
    };
    let trimmed = crate::pytext::trim(raw);
    match trimmed.parse::<u64>() {
        Ok(value) => value.clamp(MIN_CLI_TIMEOUT_SECONDS, MAX_CLI_TIMEOUT_SECONDS),
        Err(_) => DEFAULT_CLI_TIMEOUT_SECONDS,
    }
}

/// The environment the child process receives.
///
/// The list is the allowlist plus the CLI's own names; nothing else is
/// inherited, and the environment starts empty.
#[must_use]
pub fn child_env(spec: &CliSpec, environ: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    for name in ENV_ALLOWLIST {
        if let Some(value) = environ.get(*name) {
            result.push(((*name).to_owned(), value.clone()));
        }
    }
    for name in spec.env_passthrough {
        if let Some(value) = environ.get(*name) {
            result.push(((*name).to_owned(), value.clone()));
        }
    }
    if !result.iter().any(|(name, _)| name == "HOME") {
        if let Some(home) = crate::boundary::home_directory() {
            result.push(("HOME".to_owned(), home.to_string_lossy().into_owned()));
        }
    }
    result
}

/// The Windows executable extensions searched when `PATHEXT` is absent.
///
/// The default is the platform's own documented order, so an npm-installed
/// `claude.cmd`/`codex.cmd`/`opencode.cmd` shim is found without the
/// operator configuring anything.
pub const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Whether a path names an executable file.
#[must_use]
pub fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The extension search order of one `PATH` lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathSearch {
    /// The extensions to try, each beginning with a dot, in order.
    pub extensions: Vec<String>,
    /// Whether a file without an extension may be used directly.
    ///
    /// Unix scripts have no extension, so the bare name comes first there;
    /// Windows only runs extension-bearing images, so an extension-less
    /// file (for example the POSIX helper npm writes beside a `.cmd` shim)
    /// must never be chosen.
    pub allow_bare: bool,
}

impl PathSearch {
    /// The search order of the platform this binary was built for.
    #[must_use]
    pub fn for_current_platform(environ: &BTreeMap<String, String>) -> Self {
        #[cfg(windows)]
        {
            Self {
                extensions: parse_pathext(environ.get("PATHEXT").map(String::as_str)),
                allow_bare: false,
            }
        }
        #[cfg(not(windows))]
        {
            let _ = environ;
            Self {
                extensions: vec![".exe".to_owned()],
                allow_bare: true,
            }
        }
    }
}

/// Parse a `PATHEXT` value into the extension search order.
///
/// A missing or blank value falls back to [`DEFAULT_PATHEXT`]. Each entry is
/// normalised to a leading dot, blanks are dropped and duplicates are
/// removed case-insensitively while the order is kept.
#[must_use]
pub fn parse_pathext(value: Option<&str>) -> Vec<String> {
    let raw = match value {
        Some(value) if !crate::pytext::trim(value).is_empty() => value,
        _ => DEFAULT_PATHEXT,
    };
    let mut result: Vec<String> = Vec::new();
    for part in raw.split(';') {
        let trimmed = crate::pytext::trim(part);
        if trimmed.is_empty() {
            continue;
        }
        let extension = if trimmed.starts_with('.') {
            trimmed.to_owned()
        } else {
            format!(".{trimmed}")
        };
        if !result
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&extension))
        {
            result.push(extension);
        }
    }
    result
}

/// The file names one program may carry in a directory.
fn candidate_names(binary: &str, search: &PathSearch) -> Vec<String> {
    let lower = binary.to_ascii_lowercase();
    let has_known_extension = search
        .extensions
        .iter()
        .any(|extension| lower.ends_with(&extension.to_ascii_lowercase()));
    let mut names: Vec<String> = Vec::new();
    if search.allow_bare || has_known_extension {
        names.push(binary.to_owned());
    }
    for extension in &search.extensions {
        names.push(format!("{binary}{extension}"));
    }
    names
}

/// Find a program on `PATH` with an injected extension order and file test.
///
/// The configured path first, then every `PATH` directory in order. The
/// injected [`PathSearch`] and `is_exec` closure keep the search pure, so
/// the Windows `PATHEXT` rules are unit-tested on every host.
#[must_use]
pub fn resolve_program_with<F>(
    spec: &CliSpec,
    environ: &BTreeMap<String, String>,
    search: &PathSearch,
    is_exec: F,
) -> Option<PathBuf>
where
    F: Fn(&Path) -> bool,
{
    if let Some(raw) = environ.get(spec.path_override_env) {
        let trimmed = crate::pytext::trim(raw);
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if is_exec(&candidate) {
                return Some(candidate);
            }
            for extension in &search.extensions {
                let with_extension = PathBuf::from(format!("{trimmed}{extension}"));
                if is_exec(&with_extension) {
                    return Some(with_extension);
                }
            }
        }
    }
    let path = environ.get("PATH")?;
    for directory in std::env::split_paths(path) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for name in candidate_names(spec.binary, search) {
            let candidate = directory.join(name);
            if is_exec(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Find the CLI: the configured path first, then `PATH`.
///
/// The configured path wins because an operator who names an executable means
/// it, even when another copy sits earlier on `PATH`.
#[must_use]
pub fn resolve_program(spec: &CliSpec, environ: &BTreeMap<String, String>) -> Option<PathBuf> {
    resolve_program_with(
        spec,
        environ,
        &PathSearch::for_current_platform(environ),
        is_executable,
    )
}

/// Whether a program is a Windows batch shim that `cmd.exe` must interpret.
#[must_use]
pub fn is_batch_shim(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
}

/// Quote one argument for the `cmd.exe` command line.
///
/// A batch shim is interpreted by `cmd.exe`, so the MSVC argv quoting that
/// `Command` applies around the whole string is not enough: cmd re-parses
/// the line, and its metacharacters (`&`, `|`, `<`, `>`, `^`, `%`, `!`,
/// parentheses, quotes) would change the command. The escaping follows the
/// well-tested `cross-spawn` algorithm: backslash runs before a quote are
/// doubled and the quote is escaped, a trailing backslash run is doubled,
/// the token is wrapped in quotes, and every cmd metacharacter is prefixed
/// with `^`.
///
/// The prompt never rides on argv (it goes to standard input), so the only
/// caller-supplied argument is a validated model id; [`cmd_shim_argv`]
/// nevertheless refuses control characters outright.
#[must_use]
pub fn escape_cmd_argument(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    let mut backslashes = 0usize;
    for character in value.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                escaped.push_str(&"\\".repeat(backslashes * 2 + 1));
                escaped.push('"');
                backslashes = 0;
            }
            other => {
                escaped.push_str(&"\\".repeat(backslashes));
                escaped.push(other);
                backslashes = 0;
            }
        }
    }
    escaped.push_str(&"\\".repeat(backslashes * 2));
    let quoted = format!("\"{escaped}\"");
    let mut out = String::with_capacity(quoted.len() * 2);
    for character in quoted.chars() {
        if matches!(
            character,
            '(' | ')'
                | '%'
                | '!'
                | '^'
                | '"'
                | '`'
                | '<'
                | '>'
                | '&'
                | '|'
                | ';'
                | ','
                | ' '
                | '*'
                | '?'
        ) {
            out.push('^');
        }
        out.push(character);
    }
    out
}

/// The `cmd.exe` argv that runs a batch shim with an argv array.
///
/// The result is `["/d", "/s", "/c", "<quoted command line>"]`. It is a
/// normal argument vector: no caller ever concatenates it into a string for
/// `sh`, and nothing in it is re-read as a file name.
///
/// # Errors
///
/// Returns a reason when the program or one argument contains a control
/// character, which would end the `cmd.exe` line early.
pub fn cmd_shim_argv(program: &Path, argv: &[String]) -> Result<Vec<String>, String> {
    let program_text = program.to_string_lossy().into_owned();
    let mut tokens: Vec<String> = Vec::with_capacity(argv.len() + 1);
    tokens.push(escape_cmd_argument(&program_text));
    for argument in argv {
        tokens.push(escape_cmd_argument(argument));
    }
    for token in &tokens {
        if token
            .chars()
            .any(|character| matches!(character, '\n' | '\r' | '\0'))
        {
            return Err("a Windows batch shim argument contains a control character".to_owned());
        }
    }
    // `/s` strips the first and last quote of the remainder, so the whole
    // line is wrapped once more: `"prog" ^"arg^" ...`.
    Ok(vec![
        "/d".to_owned(),
        "/s".to_owned(),
        "/c".to_owned(),
        format!("\"{}\"", tokens.join(" ")),
    ])
}

/// The executable that the probe and the turn share.
pub fn program_or_reason(
    spec: &CliSpec,
    environ: &BTreeMap<String, String>,
) -> Result<PathBuf, ProviderFailure> {
    resolve_program(spec, environ).ok_or_else(|| {
        ProviderFailure::Unavailable(format!(
            "the {} was not found on PATH or at {}",
            spec.name, spec.path_override_env
        ))
    })
}

/// One CLI invocation.
///
/// The struct keeps the run's inputs together: the argv is fixed, the
/// payload goes to standard input, the environment is sanitized and the
/// working directory is private.
pub struct CliInvocation<'a> {
    /// The CLI being run.
    pub spec: &'a CliSpec,
    /// The executable.
    pub program: &'a Path,
    /// The fixed argument vector. The payload is never part of it.
    pub argv: &'a [String],
    /// The payload written to the child's standard input.
    pub stdin: &'a [u8],
    /// The wall-clock budget.
    pub timeout: Duration,
    /// The environment view the sanitized child environment is built from.
    pub environ: &'a BTreeMap<String, String>,
    /// The private working directory.
    pub workdir: &'a Path,
    /// Extra environment names for the child; Kala's own sandboxing
    /// instructions, never a credential.
    pub extra_env: &'a [(String, String)],
}

/// A running CLI turn's process tree.
///
/// Every turn is started in its own process group (Unix
/// `process_group(0)`, Windows `CREATE_NEW_PROCESS_GROUP`), so the group id
/// is the child's pid. The guard kills the whole tree on a timeout, on an
/// error and when the request future is dropped because the client
/// disconnected or cancelled: dropping the future drops this guard before
/// `kill_on_drop` reaps the direct child.
struct ProcessTree {
    pid: u32,
    armed: bool,
}

impl ProcessTree {
    fn attach(pid: Option<u32>) -> Self {
        Self {
            pid: pid.unwrap_or(0),
            armed: pid.is_some(),
        }
    }

    /// Stop watching: the turn finished on its own and the child was reaped.
    fn disarm(&mut self) {
        self.armed = false;
    }

    /// Kill the whole group, once.
    fn kill(&mut self) {
        if self.armed {
            kill_process_tree(self.pid);
            self.armed = false;
        }
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Kill one process and its whole process group.
///
/// Unix: `kill(-pgid, SIGKILL)` then the direct pid as a backstop, because a
/// CLI that deliberately creates its own session leaves the group. Windows:
/// `taskkill /T /F`, which walks the parent/child tree; a Job Object would
/// be stronger (`kala-process` uses one for sandboxed tool runs) but this
/// crate has no Windows execution proof, so the documented OS tool is used
/// and `CREATE_NEW_PROCESS_GROUP` keeps the kill out of the server's own
/// console group.
fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal, kill_process, kill_process_group};
        let Ok(raw) = i32::try_from(pid) else {
            return;
        };
        let Some(pid) = Pid::from_raw(raw) else {
            return;
        };
        let _ = kill_process_group(pid, Signal::KILL);
        let _ = kill_process(pid, Signal::KILL);
    }
    #[cfg(windows)]
    {
        // `taskkill /T` walks the parent/child tree and `/F` forces it.
        let mut command = std::process::Command::new("taskkill");
        command.args(["/T", "/F", "/PID", &pid.to_string()]);
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        if let Ok(mut child) = command.spawn() {
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            let _ = child.kill();
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

/// The Windows `CREATE_NEW_PROCESS_GROUP` creation flag.
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Build the command for one turn, through `cmd.exe` for a batch shim.
fn build_command(
    spec: &CliSpec,
    program: &Path,
    argv: &[String],
    environ: &BTreeMap<String, String>,
    extra_env: &[(String, String)],
    workdir: &Path,
) -> Result<Command, ProviderFailure> {
    let mut command = if is_batch_shim(program) {
        #[cfg(windows)]
        {
            let shim_argv = cmd_shim_argv(program, argv).map_err(|reason| {
                ProviderFailure::Subprocess(format!(
                    "the {} could not be started: {reason}",
                    spec.name
                ))
            })?;
            // The prompt stays on standard input; `cmd.exe` hands it to the
            // shim, and no caller text is concatenated into the line.
            let mut command = Command::new("cmd.exe");
            command.args(&shim_argv);
            command
        }
        #[cfg(not(windows))]
        {
            return Err(ProviderFailure::Subprocess(format!(
                "the {} resolved to a Windows batch shim (.cmd/.bat), which this platform cannot run",
                spec.name
            )));
        }
    } else {
        let mut command = Command::new(program);
        command.args(argv);
        command
    };
    command
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        // A new process group whose id is the child's pid, so the group can
        // be killed as a unit. `setsid` is not needed: the child keeps the
        // server's session, which is what a CLI expects.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    command.env_clear();
    for (name, value) in child_env(spec, environ) {
        command.env(name, value);
    }
    for (name, value) in extra_env {
        command.env(name, value);
    }
    Ok(command)
}

/// Run one CLI with a fixed argv, a sanitized environment, a private working
/// directory and the payload on standard input.
///
/// # Errors
///
/// Returns the typed provider failure the Studio route maps onto a sanitized
/// status. The child's own text is bounded and never carries a credential:
/// the CLI's output is the only thing that can reach the operator.
pub async fn run(invocation: CliInvocation<'_>) -> Result<CliRun, ProviderFailure> {
    let CliInvocation {
        spec,
        program,
        argv,
        stdin,
        timeout,
        environ,
        workdir,
        extra_env,
    } = invocation;
    let mut command = build_command(spec, program, argv, environ, extra_env, workdir)?;

    let mut child = match spawn_child(&mut command, spec).await {
        Ok(child) => child,
        Err(failure) => return Err(failure),
    };
    let mut tree = ProcessTree::attach(child.id());
    let stdin_pipe = child.stdin.take();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let payload = stdin.to_vec();
    let writer = tokio::spawn(async move {
        let Some(mut pipe) = stdin_pipe else {
            return;
        };
        let _ = pipe.write_all(&payload).await;
        let _ = pipe.shutdown().await;
    });

    let outcome = tokio::time::timeout(timeout, async {
        let (stdout, stderr) = tokio::join!(
            read_capped(stdout_pipe, MAX_CLI_STDOUT_BYTES),
            read_capped(stderr_pipe, MAX_CLI_STDERR_BYTES)
        );
        let status = child.wait().await.ok();
        (stdout, stderr, status)
    })
    .await;

    match outcome {
        Ok((Ok(stdout), Ok(stderr), status)) => {
            let _ = writer.await;
            // The child exited on its own; do not signal the group.
            tree.disarm();
            Ok(CliRun {
                status: status.and_then(|status| status.code()),
                stdout,
                stderr,
            })
        }
        Ok((Err(_), _, _)) | Ok((_, Err(_), _)) => {
            let _ = writer.await;
            tree.kill();
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            Err(ProviderFailure::Internal(format!(
                "the {} produced more output than the bounded size",
                spec.name
            )))
        }
        Err(_) => {
            tree.kill();
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            let _ = writer.await;
            Err(ProviderFailure::Timeout(format!(
                "the {} exceeded its bounded timeout",
                spec.name
            )))
        }
    }
}

/// Start the child, retrying the "text file busy" refusal.
///
/// The kernel refuses to execute a file that is still open for writing, and
/// the release of the last reference to a file is deferred: a program that
/// writes an executable and starts it in the same breath can be refused with
/// `ETXTBSY` even though the writer has closed its handle. The refusal is
/// transient, so the start is retried a few times before the turn is
/// refused. A CLI that an installer is replacing at that exact moment is
/// the realistic case.
async fn spawn_child(
    command: &mut Command,
    spec: &CliSpec,
) -> Result<tokio::process::Child, ProviderFailure> {
    let mut attempt = 0u32;
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && attempt < MAX_BUSY_START_ATTEMPTS =>
            {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(BUSY_START_BACKOFF_MILLIS)).await;
            }
            Err(error) => {
                return Err(ProviderFailure::Subprocess(format!(
                    "the {} could not be started ({:?})",
                    spec.name,
                    error.kind()
                )));
            }
        }
    }
}

/// Read one pipe, refusing to grow past the cap.
async fn read_capped<R>(pipe: Option<R>, cap: usize) -> Result<Vec<u8>, ()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(mut pipe) = pipe else {
        return Ok(Vec::new());
    };
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8_192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Ok(count) => {
                if buffer.len() + count > cap {
                    return Err(());
                }
                buffer.extend_from_slice(&chunk[..count]);
            }
            Err(_) => return Err(()),
        }
    }
    Ok(buffer)
}

/// The bounded, control-character-free excerpt of a CLI failure.
#[must_use]
pub fn failure_excerpt(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(MAX_CLI_MESSAGE_LENGTH);
    for character in text.chars() {
        if out.chars().count() >= MAX_CLI_MESSAGE_LENGTH {
            break;
        }
        if character.is_control() && !matches!(character, '\n' | '\t') {
            out.push(' ');
            continue;
        }
        out.push(character);
    }
    out.trim().to_owned()
}

/// The bounded, control-character-free excerpt of a CLI failure text.
#[must_use]
pub fn failure_text(text: &str) -> String {
    failure_excerpt(text.as_bytes())
}

/// The bounded version token of a CLI's own version output.
///
/// Only a token that looks like a version survives, so a CLI that prints
/// something unexpected cannot smuggle text into a response. The first
/// version-shaped word of the first line wins; a CLI that prints no version
/// at all yields an empty token.
#[must_use]
pub fn sanitize_version(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let first_line = text.lines().next().unwrap_or("");
    let mut fallback = String::new();
    for token in first_line.split_whitespace() {
        let clean: String = token
            .chars()
            .filter(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '+' | '-')
            })
            .collect();
        if clean.is_empty() {
            continue;
        }
        if clean
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            return clean.chars().take(64).collect();
        }
        if fallback.is_empty() {
            fallback = clean.chars().take(64).collect();
        }
    }
    fallback
}

/// One cached probe verdict.
#[derive(Clone, Debug)]
struct CachedProbe {
    fetched_at_epoch: u64,
    program: Option<PathBuf>,
    probe: CliProbe,
}

/// The probe cache plus the ids whose background refresh is running.
#[derive(Default)]
struct ProbeState {
    cache: BTreeMap<&'static str, CachedProbe>,
    /// The `(provider id, resolved program)` pairs being refreshed.
    ///
    /// The program is part of the key so an operator who renames or
    /// replaces a CLI (a different resolved path) does not have to wait for
    /// the previous environment's refresh to drain.
    refreshing: std::collections::BTreeSet<(&'static str, Option<PathBuf>)>,
}

fn probe_cache() -> &'static Mutex<ProbeState> {
    static CACHE: OnceLock<Mutex<ProbeState>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ProbeState::default()))
}

/// Drop every cached probe verdict.
///
/// The tests call this so one case never sees another's verdict.
pub fn clear_probe_cache() {
    let mut state = probe_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.cache.clear();
    state.refreshing.clear();
}

fn lock_probe_state() -> std::sync::MutexGuard<'static, ProbeState> {
    probe_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn is_fresh(entry: &CachedProbe, now: u64) -> bool {
    now.saturating_sub(entry.fetched_at_epoch) < CLI_PROBE_TTL_SECONDS
}

fn store_probe(spec: &CliSpec, program: Option<PathBuf>, verdict: &CliProbe) {
    let mut state = lock_probe_state();
    state.cache.insert(
        spec.id,
        CachedProbe {
            fetched_at_epoch: current_epoch_seconds(),
            program: program.clone(),
            probe: verdict.clone(),
        },
    );
    state.refreshing.remove(&(spec.id, program));
}

/// Start one background refresh when none is running for this CLI and
/// program.
fn start_refresh(spec: &CliSpec, environ: &BTreeMap<String, String>, program: Option<PathBuf>) {
    let mut state = lock_probe_state();
    if !state.refreshing.insert((spec.id, program.clone())) {
        return;
    }
    drop(state);
    let spec = *spec;
    let environ = environ.clone();
    tokio::spawn(async move {
        let verdict = probe_with_deadline(&spec, &environ, program.clone()).await;
        store_probe(&spec, program, &verdict);
    });
}

/// The availability probe of one subscription CLI.
///
/// The probe runs the CLI's version command and, when the CLI has one, its
/// authentication-status command. Only the exit status of the status command
/// is read: the CLI's own text never leaves this function, so no account
/// name, address or token can reach a response.
///
/// The verdict is cached for [`CLI_PROBE_TTL_SECONDS`]: the provider list is
/// polled by the Studio UI, and the CLI must not be launched on every poll.
/// This is the blocking form; the routes use [`probe_cached`], which never
/// waits for a CLI.
pub async fn probe(spec: &CliSpec, environ: &BTreeMap<String, String>) -> CliProbe {
    let program = resolve_program(spec, environ);
    let now = current_epoch_seconds();
    {
        let state = lock_probe_state();
        if let Some(entry) = state.cache.get(spec.id) {
            if is_fresh(entry, now) && entry.program == program {
                return entry.probe.clone();
            }
        }
    }
    let verdict = probe_with_deadline(spec, environ, program.clone()).await;
    store_probe(spec, program, &verdict);
    verdict
}

/// The cached verdict of one CLI, or the honest "checking" row.
///
/// A cached verdict is returned at once, even when it is stale; a stale or
/// missing verdict starts a background refresh, so no route ever waits for
/// a CLI process. The refresh is capped by
/// [`CLI_PROBE_TIMEOUT_SECONDS`] and kills its own process group.
pub(crate) fn probe_cached(spec: &CliSpec, environ: &BTreeMap<String, String>) -> CliProbe {
    let program = resolve_program(spec, environ);
    let now = current_epoch_seconds();
    let state = lock_probe_state();
    if let Some(entry) = state.cache.get(spec.id) {
        if entry.program == program {
            if is_fresh(entry, now) {
                return entry.probe.clone();
            }
            let stale = entry.probe.clone();
            drop(state);
            start_refresh(spec, environ, program);
            return stale;
        }
    }
    drop(state);
    start_refresh(spec, environ, program);
    CliProbe::checking(spec.id)
}

/// The three subscription verdicts, waiting at most the route budget.
///
/// The first call starts the refreshes and waits up to
/// [`PROBE_ROUTE_BUDGET_MILLIS`] for them: a normal CLI answers inside that
/// window, and a hung one leaves the row in the honest "checking" state
/// instead of blocking the response. Later calls answer from the cache.
pub async fn probe_subscription_clis_cached(
    specs: &[CliSpec],
    environ: &BTreeMap<String, String>,
) -> Vec<CliProbe> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(PROBE_ROUTE_BUDGET_MILLIS);
    let mut probes: Vec<CliProbe> = specs
        .iter()
        .map(|spec| probe_cached(spec, environ))
        .collect();
    while probes.iter().any(|probe| probe.checking) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(PROBE_POLL_MILLIS)).await;
        probes = specs
            .iter()
            .map(|spec| probe_cached(spec, environ))
            .collect();
    }
    probes
}

/// One probe under the per-probe cap.
///
/// A CLI that never answers leaves a resolved binary with no verdict; the
/// honest answer is "installed, but its availability check did not finish",
/// never "unavailable" and never a guess that it is signed in.
async fn probe_with_deadline(
    spec: &CliSpec,
    environ: &BTreeMap<String, String>,
    program: Option<PathBuf>,
) -> CliProbe {
    if program.is_none() {
        return probe_uncached(spec, environ, None).await;
    }
    let budget = Duration::from_secs(CLI_PROBE_TIMEOUT_SECONDS);
    match tokio::time::timeout(budget, probe_uncached(spec, environ, program)).await {
        Ok(verdict) => verdict,
        Err(_) => CliProbe {
            id: spec.id,
            installed: true,
            authenticated: false,
            checking: false,
            reason: format!(
                "the {} is installed but its availability check did not finish within {} seconds",
                spec.name, CLI_PROBE_TIMEOUT_SECONDS
            ),
        },
    }
}

async fn probe_uncached(
    spec: &CliSpec,
    environ: &BTreeMap<String, String>,
    program: Option<PathBuf>,
) -> CliProbe {
    let Some(program) = program else {
        return CliProbe {
            id: spec.id,
            installed: false,
            authenticated: false,
            checking: false,
            reason: format!(
                "the {} was not found on PATH or at {}",
                spec.name, spec.path_override_env
            ),
        };
    };
    let workdir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(_) => {
            return CliProbe {
                id: spec.id,
                installed: false,
                authenticated: false,
                checking: false,
                reason: format!(
                    "a private working directory for the {} could not be created",
                    spec.name
                ),
            };
        }
    };
    let version_args: Vec<String> = spec
        .version_args
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect();
    let version = run(CliInvocation {
        spec,
        program: &program,
        argv: &version_args,
        stdin: b"",
        timeout: Duration::from_secs(CLI_PROBE_TIMEOUT_SECONDS),
        environ,
        workdir: workdir.path(),
        extra_env: &[],
    })
    .await;
    let version = match version {
        Ok(run) if run.succeeded() => run,
        Ok(run) => {
            return CliProbe {
                id: spec.id,
                installed: false,
                authenticated: false,
                checking: false,
                reason: format!(
                    "the {} did not report a usable version: it exited with status {}",
                    spec.name,
                    run.status.unwrap_or(-1)
                ),
            };
        }
        Err(ProviderFailure::Timeout(_)) => {
            // The file exists and was started; it simply never answered.
            return CliProbe {
                id: spec.id,
                installed: true,
                authenticated: false,
                checking: false,
                reason: format!(
                    "the {} is installed but did not answer its version check within {} seconds",
                    spec.name, CLI_PROBE_TIMEOUT_SECONDS
                ),
            };
        }
        Err(_) => {
            return CliProbe {
                id: spec.id,
                installed: false,
                authenticated: false,
                checking: false,
                reason: format!(
                    "the {} did not report a usable version: it could not be started",
                    spec.name
                ),
            };
        }
    };
    let token = sanitize_version(&version.stdout);
    let label = if token.is_empty() {
        spec.name.to_owned()
    } else {
        format!("{} {token}", spec.name)
    };
    let auth_args: Vec<String> = spec
        .auth_args
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect();
    let auth = run(CliInvocation {
        spec,
        program: &program,
        argv: &auth_args,
        stdin: b"",
        timeout: Duration::from_secs(CLI_PROBE_TIMEOUT_SECONDS),
        environ,
        workdir: workdir.path(),
        extra_env: &[],
    })
    .await;
    let authenticated = matches!(&auth, Ok(run) if run.succeeded());
    let reason = if authenticated {
        format!("{label} is installed and signed in")
    } else {
        format!("{label} is installed but reports that it is not signed in")
    };
    CliProbe {
        id: spec.id,
        installed: true,
        authenticated,
        checking: false,
        reason,
    }
}

/// The refusal the route shows when the operator did not consent.
#[must_use]
pub fn consent_message(provider_id: &str) -> String {
    format!("provider={provider_id} requires allow_remote=true at request time")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn spec() -> CliSpec {
        CliSpec {
            id: "probe_test",
            name: "Probe Test CLI",
            binary: "probe-test",
            path_override_env: "KALA_PROBE_TEST_PATH",
            version_args: &["--version"],
            auth_args: &["login", "status"],
            env_passthrough: &[],
        }
    }

    fn environ(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn pathext_defaults_and_normalises() {
        assert_eq!(
            parse_pathext(None),
            vec![".COM", ".EXE", ".BAT", ".CMD"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
        assert_eq!(parse_pathext(Some("")), parse_pathext(None));
        assert_eq!(
            parse_pathext(Some("exe; .CMD ;exe;;.Bat")),
            vec![".exe", ".CMD", ".Bat"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
    }

    /// The Windows npm-shim case: `probe-test` exists as a POSIX helper, the
    /// runnable file is `probe-test.CMD`.
    #[test]
    fn a_windows_search_prefers_the_cmd_shim_over_the_bare_file() {
        // Windows file systems match names case-insensitively; the fake
        // set spells the names the search builds.
        let files: BTreeSet<PathBuf> = ["/shims/probe-test", "/shims/probe-test.CMD"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let search = PathSearch {
            extensions: parse_pathext(Some(".COM;.EXE;.BAT;.CMD")),
            allow_bare: false,
        };
        let found =
            resolve_program_with(&spec(), &environ(&[("PATH", "/shims")]), &search, |path| {
                files.contains(path)
            });
        assert_eq!(found, Some(PathBuf::from("/shims/probe-test.CMD")));
    }

    #[test]
    fn a_windows_search_keeps_pathext_order() {
        let files: BTreeSet<PathBuf> = ["/shims/probe-test.EXE", "/shims/probe-test.BAT"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let search = PathSearch {
            extensions: parse_pathext(Some(".BAT;.EXE")),
            allow_bare: false,
        };
        let found =
            resolve_program_with(&spec(), &environ(&[("PATH", "/shims")]), &search, |path| {
                files.contains(path)
            });
        assert_eq!(found, Some(PathBuf::from("/shims/probe-test.BAT")));
    }

    #[test]
    fn a_unix_search_accepts_a_bare_program_and_a_dot_exe() {
        let bare: BTreeSet<PathBuf> = [PathBuf::from("/bin/probe-test")].into_iter().collect();
        let search = PathSearch {
            extensions: vec![".exe".to_owned()],
            allow_bare: true,
        };
        assert_eq!(
            resolve_program_with(&spec(), &environ(&[("PATH", "/bin")]), &search, |path| bare
                .contains(path),),
            Some(PathBuf::from("/bin/probe-test"))
        );
        let exe: BTreeSet<PathBuf> = [PathBuf::from("/bin/probe-test.exe")].into_iter().collect();
        assert_eq!(
            resolve_program_with(&spec(), &environ(&[("PATH", "/bin")]), &search, |path| exe
                .contains(path),),
            Some(PathBuf::from("/bin/probe-test.exe"))
        );
    }

    #[test]
    fn an_override_wins_and_gets_the_extension_appended() {
        let files: BTreeSet<PathBuf> = [
            PathBuf::from("/elsewhere/probe-test.CMD"),
            PathBuf::from("/shims/probe-test.cmd"),
        ]
        .into_iter()
        .collect();
        let search = PathSearch {
            extensions: parse_pathext(Some(".CMD")),
            allow_bare: false,
        };
        let found = resolve_program_with(
            &spec(),
            &environ(&[
                ("PATH", "/shims"),
                ("KALA_PROBE_TEST_PATH", "/elsewhere/probe-test"),
            ]),
            &search,
            |path| files.contains(path),
        );
        assert_eq!(found, Some(PathBuf::from("/elsewhere/probe-test.CMD")));
    }

    #[test]
    fn a_relative_or_missing_override_falls_through_to_path() {
        let files: BTreeSet<PathBuf> = [PathBuf::from("/shims/probe-test.CMD")]
            .into_iter()
            .collect();
        let search = PathSearch {
            extensions: parse_pathext(Some(".CMD")),
            allow_bare: false,
        };
        for override_value in ["   ", "relative/probe-test"] {
            let found = resolve_program_with(
                &spec(),
                &environ(&[("PATH", "/shims"), ("KALA_PROBE_TEST_PATH", override_value)]),
                &search,
                |path| files.contains(path),
            );
            assert_eq!(
                found,
                Some(PathBuf::from("/shims/probe-test.CMD")),
                "override {override_value:?}"
            );
        }
    }

    #[test]
    fn batch_shim_detection_is_case_insensitive() {
        assert!(is_batch_shim(Path::new("C:/npm/codex.CMD")));
        assert!(is_batch_shim(Path::new("C:/npm/codex.bat")));
        assert!(!is_batch_shim(Path::new("C:/npm/codex.exe")));
        assert!(!is_batch_shim(Path::new("C:/npm/codex")));
    }

    #[test]
    fn cmd_quoting_neutralises_metacharacters() {
        // The prompt never rides on argv, but a model id can carry odd text.
        assert_eq!(escape_cmd_argument("plain"), "^\"plain^\"");
        assert_eq!(escape_cmd_argument("two words"), "^\"two^ words^\"");
        assert_eq!(escape_cmd_argument("a&b|c"), "^\"a^&b^|c^\"");
        assert_eq!(escape_cmd_argument("a\"b"), "^\"a\\^\"b^\"");
        assert_eq!(escape_cmd_argument("C:\\dir\\"), "^\"C:\\dir\\\\^\"");
        assert_eq!(escape_cmd_argument("100%"), "^\"100^%^\"");
        assert_eq!(escape_cmd_argument("(x);y,z*?"), "^\"^(x^)^;y^,z^*^?^\"");
    }

    #[test]
    fn cmd_shim_argv_is_an_argument_vector() {
        let argv = cmd_shim_argv(
            Path::new("C:/Users/me/AppData/Roaming/npm/codex.cmd"),
            &["exec".to_owned(), "--model".to_owned(), "gpt-5".to_owned()],
        )
        .unwrap();
        assert_eq!(argv[0], "/d");
        assert_eq!(argv[1], "/s");
        assert_eq!(argv[2], "/c");
        assert!(
            argv[3].starts_with("\"^\"C:/Users/me/AppData/Roaming/npm/codex.cmd^\""),
            "{}",
            argv[3]
        );
        assert!(argv[3].contains("^\"exec^\""), "{}", argv[3]);
        assert_eq!(argv.len(), 4);
    }

    #[test]
    fn cmd_shim_argv_refuses_a_control_character() {
        let error = cmd_shim_argv(Path::new("C:/npm/codex.cmd"), &["a\nb".to_owned()])
            .expect_err("a newline would end the cmd line early");
        assert!(error.contains("control character"), "{error}");
    }

    #[test]
    fn a_checking_row_is_not_usable_and_never_available() {
        let probe = CliProbe::checking("probe_test");
        assert!(probe.checking);
        assert!(!probe.usable());
        assert_eq!(probe.reason, PROBE_CHECKING_REASON);
    }

    /// Windows-only: the real `PATHEXT` search finds an extension-bearing
    /// image on the real file system, and a `.cmd` shim runs through
    /// `cmd.exe` with the argv array and the prompt on standard input.
    #[cfg(windows)]
    mod windows {
        use super::*;

        #[test]
        fn the_real_pathext_search_finds_cmd_exe() {
            let environ: BTreeMap<String, String> = std::env::vars().collect();
            let search = PathSearch::for_current_platform(&environ);
            let found = resolve_program_with(&spec_for("cmd"), &environ, &search, is_executable);
            let found = found.expect("cmd.exe is on every Windows PATH");
            assert!(found.is_file());
            assert_eq!(
                found
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("exe")
            );
        }

        fn spec_for(binary: &'static str) -> CliSpec {
            CliSpec {
                id: "windows_probe",
                name: "Windows Probe",
                binary,
                path_override_env: "KALA_WINDOWS_PROBE_PATH",
                version_args: &["/?"],
                auth_args: &[],
                env_passthrough: &[],
            }
        }

        #[tokio::test]
        async fn a_cmd_shim_receives_the_argv_and_the_prompt_on_stdin() {
            let work = tempfile::tempdir().unwrap();
            let shim = work.path().join("probe shim.cmd");
            // %* carries the argv after /c, `more` copies standard input.
            std::fs::write(
                &shim,
                "@echo off\r\n\
                 echo %* > \"%~dp0argv.txt\"\r\n\
                 more > \"%~dp0stdin.txt\"\r\n\
                 exit /b 0\r\n",
            )
            .unwrap();
            let spec = spec_for("probe shim");
            let argv = vec!["exec".to_owned(), "two words".to_owned()];
            let environ: BTreeMap<String, String> = std::env::vars().collect();
            let run = run(CliInvocation {
                spec: &spec,
                program: &shim,
                argv: &argv,
                stdin: b"the prompt",
                timeout: Duration::from_secs(20),
                environ: &environ,
                workdir: work.path(),
                extra_env: &[],
            })
            .await
            .expect("the cmd shim runs");
            assert!(run.succeeded(), "{run:?}");
            let recorded =
                std::fs::read_to_string(work.path().join("argv.txt")).unwrap_or_default();
            assert!(recorded.contains("exec"), "{recorded}");
            assert!(recorded.contains("two words"), "{recorded}");
            let stdin = std::fs::read_to_string(work.path().join("stdin.txt")).unwrap_or_default();
            assert!(stdin.contains("the prompt"), "{stdin}");
        }
    }

    #[tokio::test]
    async fn a_cached_probe_never_waits_for_a_hanging_cli() {
        use crate::providers::{claude_cli, codex_cli};
        clear_probe_cache();
        let work = tempfile::tempdir().unwrap();
        let program = work.path().join("probe-test");
        std::fs::write(&program, "#!/bin/sh\nsleep 300\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let environ = environ(&[
            ("PATH", "/usr/bin:/bin"),
            ("HOME", "/tmp"),
            (
                claude_cli::SPEC.path_override_env,
                &program.display().to_string(),
            ),
        ]);
        let started = std::time::Instant::now();
        let probes =
            probe_subscription_clis_cached(&[claude_cli::SPEC, codex_cli::SPEC], &environ).await;
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(1_000),
            "the route waited {elapsed:?} for a hanging CLI"
        );
        // The fake hangs, so its first answer is the honest checking row.
        let claude = probes
            .iter()
            .find(|probe| probe.id == claude_cli::PROVIDER_ID)
            .expect("the claude row");
        assert!(claude.checking, "{claude:?}");
        clear_probe_cache();
    }
}
