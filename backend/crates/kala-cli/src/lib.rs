//! The Kala local command line.
//!
//! Presentation only. Every command reads validated settings from the
//! environment (plus the root `--data-dir` override), opens the same local
//! data directory the API uses, prints one JSON document to stdout, and
//! returns an exit code. It holds no engineering rule, decides no approval on
//! its own, and never opens an outbound connection.
//!
//! This crate is the native port of `kala_agent.cli.main` (and the package's
//! `__init__`/`__main__` entry points). It calls the accepted
//! `kala-application` composition root and the accepted `kala-api`
//! configuration, boundary and probe surfaces READ-ONLY.
//!
//! # `serve`
//!
//! The Python `serve` command mints a launch token, builds the FastAPI
//! application, and binds the loopback interface. The native command does the
//! same over the accepted `kala-api` routes and the accepted `kala-application`
//! composition root: it validates the loopback target, mints the token to
//! `<data-dir>/api-token` with owner-only permissions, and calls the launch
//! entry point of the accepted `kala-api/server` crate. The summary is printed
//! on an interrupt, and a terminate signal stops the process the way the
//! Python server's signal handling stops it.
//!
//! # Exit codes
//!
//! * `0` the requested operation completed;
//! * `1` the operation could not be completed (bad input, unknown identifier,
//!   missing checkpoint, unusable local data directory, a server that could
//!   not start for a reason other than its bind);
//! * `2` a usage error, raised by the argparse-compatible parser;
//! * `3` the operation completed but the outcome is a failure: a reference
//!   evaluation that violates a hard constraint, or a workflow that ended
//!   failed.
//!
//! A credential-shaped value in the environment is never printed: this module
//! reports provider and model identity, not endpoints or API keys. The one
//! secret it will print is the API launch token, and only when `token` is
//! asked for by name.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kala_api::config::{Settings, SettingsError, settings_from_env};
use kala_application::{ApplicationError, KalaApplication, REVISION_DECISIONS, View};
use kala_json::CanonicalValue;
use num_bigint::BigInt;

pub mod help;
pub mod json;
pub mod parser;

pub use parser::{Command, Invocation, Outcome, UsageError};

/// The program name, matching the Python console script.
pub const PROGRAM_NAME: &str = "kala-agent";
/// The requested operation completed.
pub const EXIT_OK: i32 = 0;
/// The operation could not be completed.
pub const EXIT_ERROR: i32 = 1;
/// A usage error was raised by the parser.
pub const EXIT_USAGE: i32 = 2;
/// The operation completed with a failed outcome.
pub const EXIT_FAILED_OUTCOME: i32 = 3;
/// The environment variable the data directory is read from.
pub const DATA_DIR_ENVIRONMENT_KEY: &str = "KALA_AGENT_DATA_DIR";
/// The largest serialized parameter-override document, in JSON bytes.
pub const MAX_PARAMETER_BYTES: usize = 16_384;
/// The longest operator reason.
pub const MAX_REASON_LENGTH: usize = 512;
/// The longest actor identifier.
pub const MAX_ACTOR_LENGTH: usize = 64;
/// The largest number of parameter overrides.
pub const MAX_PARAMETERS_ENTRIES: usize = 256;
/// The length of a SHA-256 digest in lowercase hexadecimal.
pub const MAX_DIGEST_LENGTH: usize = 64;
/// The default journal page size.
pub const DEFAULT_EVENT_LIMIT: i64 = 50;
/// The largest accepted journal page size.
pub const MAX_EVENT_LIMIT: i64 = 1_000;
/// The largest accepted exclusive journal bound.
pub const MAX_AFTER_SEQUENCE: i64 = 1_000_000_000;
/// The decisions a prepared revision commit accepts.
pub const REVISION_DECISIONS_LIST: [&str; 2] = ["approve", "reject"];
/// The default revision-commit decision.
pub const DEFAULT_REVISION_DECISION: &str = "approve";
/// The loopback host `serve` binds by default.
pub const SERVE_HOST: &str = "127.0.0.1";
/// The default `serve` port.
pub const SERVE_PORT: i64 = 8000;
/// The lowest accepted `serve` port.
pub const MIN_SERVE_PORT: i64 = 1;
/// The highest accepted `serve` port.
pub const MAX_SERVE_PORT: i64 = 65_535;
/// The Python optional server module name.
pub const UVICORN_MODULE_NAME: &str = "uvicorn";
/// The uvicorn log level the Python command passes.
pub const SERVE_LOG_LEVEL: &str = "warning";
/// The Python message for a missing optional server dependency.
pub const SERVE_UNAVAILABLE_MESSAGE: &str = "the serve command needs the uvicorn server, which is an optional runtime dependency; install it with: pip install 'kala-agent[serve]'";
/// The refusal of a routable or wildcard bind address.
pub const NON_LOOPBACK_HOST_MESSAGE: &str =
    "serve binds the loopback interface only; a routable or wildcard address is refused";
/// The exit code a Python launch reports when its server cannot start.
///
/// Uvicorn raises `STARTUP_FAILURE` (`3`) from its own startup path; the
/// command line never catches it, so the process exits with that code.
pub const EXIT_SERVE_STARTUP_FAILURE: i32 = 3;
/// The native `serve` refusal for a composition root the caller owns.
///
/// The native router needs an `Arc` of the root it serves; a borrowed root
/// cannot be shared with it. The binary never takes this path, because it
/// always opens its own root.
pub const SERVE_INJECTED_APPLICATION_MESSAGE: &str = "serve needs the application this process opened; an application supplied by a caller cannot be shared with the native server";
/// The disclaimer printed by `init`.
pub const DISCLAIMER: &str = "Bounded surrogate reference arithmetic only; not CAD, CAE, a code check, or an engineering certification. No authentication and no network access.";
/// The exit-code legend in the root help.
pub const EPILOG: &str =
    "Exit codes: 0 completed, 1 error, 2 usage, 3 completed with a failed outcome.";

/// The loopback address the local API is served on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeTarget {
    /// The bind host.
    pub host: String,
    /// The bind port.
    pub port: i64,
}

impl ServeTarget {
    /// The `http://host:port` URL, bracketing an IPv6 literal.
    #[must_use]
    pub fn url(&self) -> String {
        let authority = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("http://{authority}:{}", self.port)
    }
}

/// Everything the `serve` command resolves before a socket is opened.
///
/// The Python plan also holds the built FastAPI application. The native plan
/// holds the resolved target and the token path; the application is the one
/// the command already opened, and its router is built by the launch entry
/// point of the accepted `kala-api/server` crate. The token value is never
/// part of this value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServePlan {
    /// The resolved loopback address.
    pub target: ServeTarget,
    /// The path the launch token was stored at.
    pub token_path: PathBuf,
}

/// Return the loopback address this command is allowed to bind.
///
/// A host is only ever a loopback interface: a wildcard or routable address is
/// refused here, before a socket exists. The port is a plain bounded TCP port.
///
/// # Errors
///
/// Fails with the Python messages for a non-loopback host or an out-of-range
/// port.
pub fn build_serve_target(
    host: Option<&str>,
    port: Option<&BigInt>,
) -> Result<ServeTarget, ApplicationError> {
    let resolved_host = match host {
        None => SERVE_HOST.to_owned(),
        Some(value) => kala_api::pytext::trim(value).to_owned(),
    };
    if !kala_api::boundary::is_loopback_host(&resolved_host) {
        return Err(ApplicationError::application(NON_LOOPBACK_HOST_MESSAGE));
    }
    let zero = BigInt::from(0);
    let resolved_port = match port {
        None => BigInt::from(SERVE_PORT),
        Some(value) => value.clone(),
    };
    if resolved_port < zero {
        return Err(ApplicationError::application(format!(
            "the port must be between {MIN_SERVE_PORT} and {MAX_SERVE_PORT}"
        )));
    }
    let upper = BigInt::from(MAX_SERVE_PORT);
    if resolved_port > upper {
        return Err(ApplicationError::application(format!(
            "the port must be between {MIN_SERVE_PORT} and {MAX_SERVE_PORT}"
        )));
    }
    let port = i64::try_from(resolved_port).map_err(|_| {
        ApplicationError::application(format!(
            "the port must be between {MIN_SERVE_PORT} and {MAX_SERVE_PORT}"
        ))
    })?;
    if port < MIN_SERVE_PORT {
        return Err(ApplicationError::application(format!(
            "the port must be between {MIN_SERVE_PORT} and {MAX_SERVE_PORT}"
        )));
    }
    Ok(ServeTarget {
        host: resolved_host,
        port,
    })
}

/// Mint a launch token and resolve the launch the Python command would serve.
///
/// The token is written to the local data directory exactly as the Python
/// command writes it, so `kala-agent token` can read it. The returned plan
/// carries the path, never the token value.
///
/// # Errors
///
/// Fails on a refused address or when the token cannot be minted.
pub fn build_serve_plan(
    application: &KalaApplication,
    host: Option<&str>,
    port: Option<&BigInt>,
) -> Result<ServePlan, ApplicationError> {
    let launch = build_serve_launch(application, host, port)?;
    Ok(ServePlan {
        target: launch.target,
        token_path: launch.token_path,
    })
}

/// The launch the `serve` command runs.
///
/// It carries the token itself, because the server must enforce the very
/// token that was minted; the public plan above deliberately drops it.
struct ServeLaunch {
    /// The resolved loopback address.
    target: ServeTarget,
    /// The path the token was stored at.
    token_path: PathBuf,
    /// The token the launch requires.
    token: String,
}

/// Resolve the target and mint the launch token, keeping the token value.
fn build_serve_launch(
    application: &KalaApplication,
    host: Option<&str>,
    port: Option<&BigInt>,
) -> Result<ServeLaunch, ApplicationError> {
    let target = build_serve_target(host, port)?;
    let data_dir = &application.settings().data_dir;
    let token = issue_launch_token(data_dir)?;
    Ok(ServeLaunch {
        target,
        token_path: data_dir.join(kala_api::boundary::API_TOKEN_FILE_NAME),
        token,
    })
}

/// Mint a launch token and store it the way the Python command stores it.
///
/// A token that cannot be stored still guards the process, held in memory
/// only, and the failure is reported by the exception class name on stderr,
/// exactly as the Python logger reports it. The token itself is never written
/// to stderr.
fn issue_launch_token(data_dir: &Path) -> Result<String, ApplicationError> {
    let token = kala_api::boundary::mint_api_token().map_err(ApplicationError::application)?;
    if let Err(error) = kala_api::boundary::store_api_token(data_dir, &token) {
        eprintln!(
            "the launch token was not stored in the local data directory: {}",
            python_error_name(&error)
        );
    }
    Ok(token)
}

/// The class name CPython would report for one filesystem failure.
fn python_error_name(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => "PermissionError",
        std::io::ErrorKind::NotFound => "FileNotFoundError",
        std::io::ErrorKind::AlreadyExists => "FileExistsError",
        std::io::ErrorKind::IsADirectory => "IsADirectoryError",
        std::io::ErrorKind::NotADirectory => "NotADirectoryError",
        _ => "OSError",
    }
}

/// The native counterpart of Python's `load_uvicorn`.
///
/// Python imports the optional ASGI server and reports a plain message when it
/// is absent. The native command has no optional server module: the server is
/// compiled into this build, so the lookup always succeeds.
///
/// # Errors
///
/// This implementation has no failure mode; the `Result` is kept so the port
/// of the command keeps the Python call order.
pub fn load_uvicorn() -> Result<(), ApplicationError> {
    Ok(())
}

/// One CLI failure: an application error, or a Python runtime class the
/// command line maps to `local_failure` (only `RecursionError`, from a
/// parameter document nested past CPython's limit).
enum CliFailure {
    Application(ApplicationError),
    LocalFailure(&'static str),
}

impl From<ApplicationError> for CliFailure {
    fn from(error: ApplicationError) -> Self {
        Self::Application(error)
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run one command and return its exit code.
///
/// `argv` is the argument list without the program name; `None` reads the
/// process arguments. A supplied `application` is never closed by this
/// function; the caller owns it. `environ` is the settings source; `None`
/// reads the process environment.
pub fn main(
    argv: Option<Vec<String>>,
    application: Option<&KalaApplication>,
    environ: Option<&BTreeMap<String, String>>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let arguments = match argv {
        Some(arguments) => arguments,
        None => process_arguments(),
    };
    match parser::parse(&arguments) {
        Outcome::Help(text) => {
            let _ = stdout.write_all(text.as_bytes());
            let _ = stdout.flush();
            EXIT_OK
        }
        Outcome::Usage(error) => {
            let _ = stderr.write_all(error.render().as_bytes());
            let _ = stderr.flush();
            EXIT_USAGE
        }
        Outcome::Run(mut invocation) => {
            match normalize_decision_fields(&mut invocation.command) {
                Ok(()) => {}
                Err(error) => {
                    // The Python constructor raises JournalIntegrityError (a
                    // RuntimeError) for a failed chain verification, and the
                    // command line reports it as `local_failure` with the class
                    // name. The native root keeps the message and returns one
                    // error type, so this one presentation mapping restores the
                    // Python class name for that exact, deterministic case.
                    if error
                        .message()
                        .starts_with("the event journal cannot be verified:")
                    {
                        let _ = emit_error(stderr, "local_failure", "JournalIntegrityError");
                    } else {
                        let _ = emit_error(stderr, error.code(), error.message());
                    }
                    return EXIT_ERROR;
                }
            }
            run_invocation(invocation, application, environ, stdout, stderr)
        }
    }
}

fn process_arguments() -> Vec<String> {
    std::env::args_os()
        .skip(1)
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

fn run_invocation(
    invocation: Invocation,
    application: Option<&KalaApplication>,
    environ: Option<&BTreeMap<String, String>>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    // `serve` hands the composition root to the native server, which needs to
    // own it. Every other command only borrows the root.
    if matches!(invocation.command, Command::Serve { .. }) {
        return run_serve_invocation(invocation, application, environ, stdout, stderr);
    }
    let mut owned: Option<KalaApplication> = None;
    let mut created = false;
    let resolved: &KalaApplication = match application {
        Some(application) => application,
        None => {
            let settings = match settings_for(invocation.data_dir.as_deref(), environ) {
                Ok(settings) => settings,
                Err(errors) => {
                    let _ = emit_error(
                        stderr,
                        "invalid_configuration",
                        &safe_configuration_message(&errors),
                    );
                    return EXIT_ERROR;
                }
            };
            created = !settings.data_dir.exists();
            match KalaApplication::open(settings) {
                Ok(application) => {
                    owned = Some(application);
                    owned.as_ref().expect("just stored")
                }
                Err(error) => {
                    emit_open_error(stderr, &error);
                    return EXIT_ERROR;
                }
            }
        }
    };

    let result = dispatch(resolved, &invocation.command, created);
    match result {
        Ok((code, payload)) => {
            if let Some(application) = &owned {
                if let Err(error) = application.close() {
                    let _ = emit_error(stderr, error.code(), error.message());
                    return EXIT_ERROR;
                }
            }
            if emit(stdout, &payload).is_err() {
                return EXIT_ERROR;
            }
            code
        }
        Err(CliFailure::Application(error)) => {
            let _ = emit_error(stderr, error.code(), error.message());
            if let Some(application) = &owned {
                let _ = application.close();
            }
            EXIT_ERROR
        }
        Err(CliFailure::LocalFailure(class_name)) => {
            let _ = emit_error(stderr, "local_failure", class_name);
            if let Some(application) = &owned {
                let _ = application.close();
            }
            EXIT_ERROR
        }
    }
}

/// Why the `serve` command stopped.
enum ServeEnd {
    /// The command finished with this exit code.
    Done(i32),
    /// The launch stopped on SIGTERM, and the process must terminate the way
    /// the Python server's signal handling terminates it.
    Terminate,
}

/// Run `serve`: open the root, resolve the target, mint the token, serve.
///
/// The order is the Python order: the command line opens the composition root
/// before the command runs, the command resolves the loopback target before it
/// looks for a server, the plan mints the token before the socket exists, and
/// the server is started last.
fn run_serve_invocation(
    invocation: Invocation,
    provided: Option<&KalaApplication>,
    environ: Option<&BTreeMap<String, String>>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let Command::Serve { host, port } = invocation.command else {
        unreachable!("only the serve command reaches this function");
    };
    if provided.is_some() {
        // The Python command resolves the target before it looks for a
        // server, and the native path refuses a borrowed root only after the
        // same target check, so a refused address is the same error.
        if let Err(error) = build_serve_target(Some(&host), Some(&port)) {
            let _ = emit_error(stderr, error.code(), error.message());
            return EXIT_ERROR;
        }
        let _ = emit_error(
            stderr,
            "application_error",
            SERVE_INJECTED_APPLICATION_MESSAGE,
        );
        return EXIT_ERROR;
    }
    let settings = match settings_for(invocation.data_dir.as_deref(), environ) {
        Ok(settings) => settings,
        Err(errors) => {
            let _ = emit_error(
                stderr,
                "invalid_configuration",
                &safe_configuration_message(&errors),
            );
            return EXIT_ERROR;
        }
    };
    let application = match KalaApplication::open(settings) {
        Ok(application) => Arc::new(application),
        Err(error) => {
            emit_open_error(stderr, &error);
            return EXIT_ERROR;
        }
    };

    let end = serve_command(&application, &host, &port, stdout, stderr);

    // The Python command closes the root in its `finally`; the launch closes
    // it at the end of the lifespan. The close is idempotent, so both paths
    // end with a released root.
    let _ = application.close();

    match end {
        ServeEnd::Done(code) => code,
        ServeEnd::Terminate => die_by_sigterm(),
    }
}

/// Report a failed composition-root constructor the way the Python CLI maps it.
fn emit_open_error(stderr: &mut dyn Write, error: &ApplicationError) {
    // The Python constructor raises JournalIntegrityError (a RuntimeError) for
    // a failed chain verification, and the command line reports it as
    // `local_failure` with the class name. The native root keeps the message
    // and returns one error type, so this one presentation mapping restores
    // the Python class name for that exact, deterministic case.
    if error
        .message()
        .starts_with("the event journal cannot be verified:")
    {
        let _ = emit_error(stderr, "local_failure", "JournalIntegrityError");
    } else {
        let _ = emit_error(stderr, error.code(), error.message());
    }
}

/// Serve one launch and describe how it stopped.
fn serve_command(
    application: &Arc<KalaApplication>,
    host: &str,
    port: &BigInt,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> ServeEnd {
    let target = match build_serve_target(Some(host), Some(port)) {
        Ok(target) => target,
        Err(error) => {
            let _ = emit_error(stderr, error.code(), error.message());
            return ServeEnd::Done(EXIT_ERROR);
        }
    };
    // Python loads the optional ASGI server here, before it mints the token.
    // The native server is compiled in, so the lookup cannot fail.
    if let Err(error) = load_uvicorn() {
        let _ = emit_error(stderr, error.code(), error.message());
        return ServeEnd::Done(EXIT_ERROR);
    }
    let launch = match build_serve_launch(
        application,
        Some(&target.host),
        Some(&BigInt::from(target.port)),
    ) {
        Ok(launch) => launch,
        Err(error) => {
            let _ = emit_error(stderr, error.code(), error.message());
            return ServeEnd::Done(EXIT_ERROR);
        }
    };
    let Some(port_number) = u16::try_from(launch.target.port).ok() else {
        let _ = emit_error(
            stderr,
            "application_error",
            "the port must be between 1 and 65535",
        );
        return ServeEnd::Done(EXIT_ERROR);
    };

    match kala_api_server::launch(
        Arc::clone(application),
        &launch.target.host,
        port_number,
        &launch.token,
    ) {
        Ok(outcome) => match outcome.stopped_by {
            // The Python server treats SIGTERM as the expected termination
            // of the process: it stops gracefully and re-raises the signal.
            kala_api_server::StopSignal::Terminate => ServeEnd::Terminate,
            kala_api_server::StopSignal::Interrupt => {
                let payload = serve_summary(&launch.target, &launch.token_path);
                match emit(stdout, &payload) {
                    Ok(()) => ServeEnd::Done(EXIT_OK),
                    Err(_) => ServeEnd::Done(EXIT_ERROR),
                }
            }
        },
        // Uvicorn reports a failed bind through its logger and exits with its
        // STARTUP_FAILURE code; the command line never wraps it in the JSON
        // error document.
        Err(kala_api_server::LaunchError::Bind { source, .. }) => {
            eprintln!("{}", uvicorn_bind_line(&launch.target, &source));
            ServeEnd::Done(EXIT_SERVE_STARTUP_FAILURE)
        }
        Err(error) => {
            let _ = emit_error(stderr, error.code(), &error.message());
            ServeEnd::Done(EXIT_ERROR)
        }
    }
}

/// The one JSON document the Python command prints when its server stops.
fn serve_summary(target: &ServeTarget, token_path: &Path) -> CanonicalValue {
    CanonicalValue::object([
        ("command", CanonicalValue::text("serve")),
        (
            "data",
            CanonicalValue::object([
                ("api_token_path", path_value(token_path)),
                ("authentication", CanonicalValue::text("bearer")),
                ("host", CanonicalValue::text(&target.host)),
                ("loopback_only", CanonicalValue::Bool(true)),
                ("port", CanonicalValue::integer(target.port)),
                ("url", CanonicalValue::text(target.url())),
            ]),
        ),
        ("status", CanonicalValue::text("stopped")),
    ])
}

/// The error line the Python ASGI server writes when its bind fails.
///
/// Uvicorn formats the failure with its default `ERROR:` prefix, the errno,
/// the address the event loop tried to bind, and the lowercased system
/// message. It writes the line to stderr and exits with `3`.
fn uvicorn_bind_line(target: &ServeTarget, error: &std::io::Error) -> String {
    let number = error.raw_os_error().unwrap_or(0);
    let message = error.to_string();
    let message = message
        .strip_suffix(&format!(" (os error {number})"))
        .unwrap_or(&message);
    let address = match target.host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(address)) => {
            format!("('{address}', {}, 0, 0)", target.port)
        }
        Ok(std::net::IpAddr::V4(address)) => format!("('{address}', {})", target.port),
        Err(_) if target.host.eq_ignore_ascii_case("localhost") => {
            format!("('127.0.0.1', {})", target.port)
        }
        Err(_) => format!("('{}', {})", target.host, target.port),
    };
    format!(
        "ERROR:    [Errno {number}] error while attempting to bind on address {address}: [errno {number}] {}",
        message.to_lowercase()
    )
}

/// Terminate the process with SIGTERM, the way the Python server does.
///
/// The tokio signal driver installs a handler and never unregisters it, so
/// the default disposition is restored before the signal is raised. A caller
/// that cannot receive the signal falls back to the shell's `128 + 15` code.
#[cfg(unix)]
fn die_by_sigterm() -> ! {
    let _ = signal_hook::low_level::emulate_default_handler(signal_hook::consts::SIGTERM);
    std::process::exit(128 + 15)
}

/// Terminate the process with the shell's `128 + 15` code on an operating
/// system whose `serve` shutdown only listens for an interrupt.
#[cfg(not(unix))]
fn die_by_sigterm() -> ! {
    std::process::exit(128 + 15)
}

fn settings_for(
    data_dir: Option<&str>,
    environ: Option<&BTreeMap<String, String>>,
) -> Result<Settings, Vec<SettingsError>> {
    let mut source: BTreeMap<String, String> = match environ {
        Some(values) => values.clone(),
        None => process_environment(),
    };
    if let Some(data_dir) = data_dir {
        source.insert(DATA_DIR_ENVIRONMENT_KEY.to_owned(), data_dir.to_owned());
    }
    settings_from_env(&source)
}

fn process_environment() -> BTreeMap<String, String> {
    let mut source = BTreeMap::new();
    for (environment_name, _) in kala_api::config::ENVIRONMENT_FIELDS {
        if let Some(value) = std::env::var_os(environment_name) {
            source.insert(
                environment_name.to_owned(),
                value.to_string_lossy().into_owned(),
            );
        }
    }
    source
}

/// Mirror Python's `_safe_configuration_message`: field names and failure
/// kinds only, never the rejected values.
fn safe_configuration_message(errors: &[SettingsError]) -> String {
    let fields = errors
        .iter()
        .take(8)
        .map(|error| {
            let location = if error.location.is_empty() {
                "settings"
            } else {
                error.location.as_str()
            };
            format!("{location}:{}", error.kind)
        })
        .collect::<Vec<_>>()
        .join(",");
    if fields.is_empty() {
        "the local configuration is invalid".to_owned()
    } else {
        format!("the local configuration is invalid: {fields}")
    }
}

fn normalize_decision_fields(command: &mut Command) -> Result<(), ApplicationError> {
    match command {
        Command::RunMechanical { actor, reason, .. }
        | Command::Approve { actor, reason, .. }
        | Command::Reject { actor, reason, .. }
        | Command::CommitRevision { actor, reason, .. } => {
            *actor = actor_identifier(actor)?;
            *reason = reason_value(reason.as_deref())?;
        }
        _ => {}
    }
    Ok(())
}

fn actor_identifier(value: &str) -> Result<String, ApplicationError> {
    let trimmed = kala_api::pytext::trim(value);
    if trimmed.is_empty() || value.chars().count() > MAX_ACTOR_LENGTH {
        return Err(ApplicationError::application(
            "the actor identifier must be nonempty and bounded",
        ));
    }
    Ok(trimmed.to_owned())
}

fn reason_value(value: Option<&str>) -> Result<Option<String>, ApplicationError> {
    match value {
        None => Ok(None),
        Some(value) if value.chars().count() > MAX_REASON_LENGTH => Err(
            ApplicationError::application("the reason is longer than the permitted length"),
        ),
        Some(value) => Ok(Some(value.to_owned())),
    }
}

fn digest_value(value: Option<&str>, field_name: &str) -> Result<Option<String>, ApplicationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let normalized = kala_api::pytext::trim(value);
    if normalized.chars().count() != MAX_DIGEST_LENGTH
        || !normalized
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
    {
        return Err(ApplicationError::application(format!(
            "{field_name} must be a lowercase SHA-256 digest"
        )));
    }
    Ok(Some(normalized.to_owned()))
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn dispatch(
    application: &KalaApplication,
    command: &Command,
    created: bool,
) -> Result<(i32, CanonicalValue), CliFailure> {
    match command {
        Command::Init => command_init(application, created),
        Command::Domains => command_domains(application),
        Command::Token => command_token(application),
        Command::Serve { .. } => {
            // `serve` needs the composition root by value; `run_invocation`
            // dispatches it before the borrowing commands are reached.
            unreachable!("serve is dispatched before the borrowing commands")
        }
        Command::Evaluate {
            domain,
            design_id,
            parameters,
            parameters_json,
        } => command_evaluate(
            application,
            domain,
            design_id.as_deref(),
            parameters,
            parameters_json.as_deref(),
        ),
        Command::RunMechanical {
            design_id,
            workflow_id,
            actor,
            ..
        } => command_run_mechanical(application, design_id, workflow_id.as_deref(), actor),
        Command::WorkflowStatus { workflow_id } => {
            command_workflow_status(application, workflow_id)
        }
        Command::Approve {
            workflow_id,
            actor,
            reason,
        } => command_decide(
            application,
            workflow_id,
            "approve",
            actor,
            reason.as_deref(),
        ),
        Command::Reject {
            workflow_id,
            actor,
            reason,
        } => command_decide(application, workflow_id, "reject", actor, reason.as_deref()),
        Command::PrepareRevision {
            design_id,
            domain,
            summary,
            parent_revision_id,
            parameters,
            parameters_json,
        } => command_prepare_revision(
            application,
            design_id,
            domain,
            summary.as_deref(),
            parent_revision_id.as_deref(),
            parameters,
            parameters_json.as_deref(),
        ),
        Command::CommitRevision {
            request_id,
            decision,
            state_digest,
            action_digest,
            actor,
            reason,
        } => command_commit_revision(
            application,
            request_id,
            decision,
            state_digest.as_deref(),
            action_digest.as_deref(),
            actor,
            reason.as_deref(),
        ),
        Command::Events {
            after_sequence,
            limit,
            workflow_id,
            event_type,
            aggregate_id,
        } => command_events(
            application,
            after_sequence,
            limit,
            workflow_id.as_deref(),
            event_type.as_deref(),
            aggregate_id.as_deref(),
        ),
    }
}

fn command_init(
    application: &KalaApplication,
    created: bool,
) -> Result<(i32, CanonicalValue), CliFailure> {
    let settings = application.settings();
    let schema_version = application
        .database()
        .schema_version()
        .map_err(|error| ApplicationError::application(error.to_string()))?;
    let tools = application
        .tool_registry()
        .tool_ids()
        .map_err(|error| ApplicationError::application(error.to_string()))?;
    let data = CanonicalValue::object([
        (
            "api_token_path",
            path_value(
                &settings
                    .data_dir
                    .join(kala_api::boundary::API_TOKEN_FILE_NAME),
            ),
        ),
        ("artifact_dir", path_value(&settings.artifact_dir())),
        ("created", CanonicalValue::Bool(created)),
        ("data_dir", path_value(&settings.data_dir)),
        ("database_path", path_value(&settings.database_path())),
        ("disclaimer", CanonicalValue::text(DISCLAIMER)),
        (
            "domains",
            CanonicalValue::array(
                application
                    .domains()
                    .iter()
                    .map(|domain| CanonicalValue::text(*domain)),
            ),
        ),
        (
            "egress_configured",
            CanonicalValue::Bool(application.egress_configured()),
        ),
        ("model", CanonicalValue::text(application.model_id())),
        (
            "non_authoritative_finding",
            CanonicalValue::text(kala_domains::NON_AUTHORITATIVE_FINDING),
        ),
        ("offline", CanonicalValue::Bool(application.offline())),
        ("provider", CanonicalValue::text(application.provider_id())),
        (
            "provider_endpoint_declared",
            CanonicalValue::Bool(application.provider_endpoint_declared()),
        ),
        (
            "runs_dir",
            path_value(application.tool_executor().run_directory()),
        ),
        ("schema_version", CanonicalValue::integer(schema_version)),
        (
            "tools",
            CanonicalValue::array(tools.iter().map(CanonicalValue::text)),
        ),
        (
            "worker_count",
            CanonicalValue::integer(i64::from(settings.worker_count)),
        ),
    ]);
    Ok((
        EXIT_OK,
        CanonicalValue::object([
            ("command", CanonicalValue::text("init")),
            ("data", data),
            ("status", CanonicalValue::text("ready")),
        ]),
    ))
}

fn command_domains(application: &KalaApplication) -> Result<(i32, CanonicalValue), CliFailure> {
    let packages = application
        .list_domain_packages()
        .iter()
        .map(View::to_canonical_value)
        .collect::<Result<Vec<_>, _>>()?;
    let workflows = application
        .list_workflow_definitions()
        .iter()
        .map(View::to_canonical_value)
        .collect::<Result<Vec<_>, _>>()?;
    let data = CanonicalValue::object([
        (
            "domains",
            CanonicalValue::array(
                application
                    .domains()
                    .iter()
                    .map(|domain| CanonicalValue::text(*domain)),
            ),
        ),
        ("packages", CanonicalValue::array(packages)),
        ("workflows", CanonicalValue::array(workflows)),
    ]);
    Ok((
        EXIT_OK,
        CanonicalValue::object([
            ("command", CanonicalValue::text("domains")),
            ("data", data),
            ("status", CanonicalValue::text("listed")),
        ]),
    ))
}

fn command_token(application: &KalaApplication) -> Result<(i32, CanonicalValue), CliFailure> {
    let data_dir = &application.settings().data_dir;
    let Some(token) = kala_api::boundary::read_api_token(data_dir) else {
        return Err(CliFailure::Application(ApplicationError::application(
            "no launch token is stored in the local data directory; start the local api first",
        )));
    };
    let data = CanonicalValue::object([
        ("authentication", CanonicalValue::text("bearer")),
        (
            "path",
            path_value(&data_dir.join(kala_api::boundary::API_TOKEN_FILE_NAME)),
        ),
        ("token", CanonicalValue::text(token)),
    ]);
    Ok((
        EXIT_OK,
        CanonicalValue::object([
            ("command", CanonicalValue::text("token")),
            ("data", data),
            ("status", CanonicalValue::text("issued")),
        ]),
    ))
}

fn command_evaluate(
    application: &KalaApplication,
    domain: &str,
    design_id: Option<&str>,
    parameters: &[String],
    parameters_json: Option<&str>,
) -> Result<(i32, CanonicalValue), CliFailure> {
    let parameters = parse_parameters(parameters, parameters_json)?;
    let report = application.evaluate_domain_design(domain, parameters.as_ref(), design_id)?;
    let status_code = if report.passed {
        EXIT_OK
    } else {
        EXIT_FAILED_OUTCOME
    };
    let status = if report.passed { "passed" } else { "failed" };
    Ok((
        status_code,
        CanonicalValue::object([
            ("command", CanonicalValue::text("evaluate")),
            ("data", report.to_canonical_value()?),
            ("status", CanonicalValue::text(status)),
        ]),
    ))
}

fn command_run_mechanical(
    application: &KalaApplication,
    design_id: &str,
    workflow_id: Option<&str>,
    actor: &str,
) -> Result<(i32, CanonicalValue), CliFailure> {
    let result = application.run_mechanical_reference_workflow(workflow_id, design_id, actor)?;
    let status = result.status.as_str();
    Ok((
        run_exit_code(status),
        CanonicalValue::object([
            ("command", CanonicalValue::text("run-mechanical")),
            ("data", result.to_canonical_value()?),
            ("status", CanonicalValue::text(status)),
        ]),
    ))
}

fn command_workflow_status(
    application: &KalaApplication,
    workflow_id: &str,
) -> Result<(i32, CanonicalValue), CliFailure> {
    let state = application.get_workflow_state(workflow_id)?;
    let status = state.status.as_str();
    Ok((
        EXIT_OK,
        CanonicalValue::object([
            ("command", CanonicalValue::text("workflow-status")),
            ("data", state.to_canonical_value()?),
            ("status", CanonicalValue::text(status)),
        ]),
    ))
}

fn command_decide(
    application: &KalaApplication,
    workflow_id: &str,
    decision: &str,
    actor: &str,
    reason: Option<&str>,
) -> Result<(i32, CanonicalValue), CliFailure> {
    let actor = actor_identifier(actor)?;
    let reason = reason_value(reason)?;
    let result = application.decide_workflow_checkpoint(
        workflow_id,
        decision,
        &actor,
        None,
        None,
        reason.as_deref(),
    )?;
    let status = result.status.as_str();
    let status_code = if decision == "reject" || run_exit_code(status) == EXIT_OK {
        EXIT_OK
    } else {
        EXIT_FAILED_OUTCOME
    };
    Ok((
        status_code,
        CanonicalValue::object([
            ("command", CanonicalValue::text(decision)),
            ("data", result.to_canonical_value()?),
            ("status", CanonicalValue::text(status)),
        ]),
    ))
}

fn command_prepare_revision(
    application: &KalaApplication,
    design_id: &str,
    domain: &str,
    summary: Option<&str>,
    parent_revision_id: Option<&str>,
    parameters: &[String],
    parameters_json: Option<&str>,
) -> Result<(i32, CanonicalValue), CliFailure> {
    let parameters = parse_parameters(parameters, parameters_json)?;
    let prepared = application.prepare_design_revision(
        design_id,
        domain,
        summary,
        parameters.as_ref(),
        parent_revision_id,
        &[],
        &[],
        None,
    )?;
    let status = prepared.status.as_str();
    Ok((
        EXIT_OK,
        CanonicalValue::object([
            ("command", CanonicalValue::text("prepare-revision")),
            ("data", prepared.to_canonical_value()?),
            ("status", CanonicalValue::text(status)),
        ]),
    ))
}

fn command_commit_revision(
    application: &KalaApplication,
    request_id: &str,
    decision: &str,
    state_digest: Option<&str>,
    action_digest: Option<&str>,
    actor: &str,
    reason: Option<&str>,
) -> Result<(i32, CanonicalValue), CliFailure> {
    if !REVISION_DECISIONS.contains(&decision) {
        return Err(CliFailure::Application(ApplicationError::application(
            format!(
                "the decision must be one of {}",
                REVISION_DECISIONS.join(", ")
            ),
        )));
    }
    let state_digest = digest_value(state_digest, "the state digest")?;
    let action_digest = digest_value(action_digest, "the action digest")?;
    let actor = actor_identifier(actor)?;
    let reason = reason_value(reason)?;
    if decision == "approve" {
        let record = application.commit_design_revision(
            request_id,
            &actor,
            state_digest.as_deref(),
            action_digest.as_deref(),
            reason.as_deref(),
            None,
        )?;
        let commit = application.get_revision_commit(request_id)?;
        let status = commit.status.as_str();
        let data = CanonicalValue::object([
            ("commit", commit.to_canonical_value()?),
            ("revision", record.to_canonical_value()?),
        ]);
        return Ok((
            EXIT_OK,
            CanonicalValue::object([
                ("command", CanonicalValue::text("commit-revision")),
                ("data", data),
                ("status", CanonicalValue::text(status)),
            ]),
        ));
    }
    let rejected = application.decide_revision_commit(
        request_id,
        "reject",
        &actor,
        state_digest.as_deref(),
        action_digest.as_deref(),
        reason.as_deref(),
        None,
    )?;
    let status = rejected.status.as_str();
    let data = CanonicalValue::object([("commit", rejected.to_canonical_value()?)]);
    Ok((
        EXIT_OK,
        CanonicalValue::object([
            ("command", CanonicalValue::text("commit-revision")),
            ("data", data),
            ("status", CanonicalValue::text(status)),
        ]),
    ))
}

fn command_events(
    application: &KalaApplication,
    after_sequence: &BigInt,
    limit: &BigInt,
    workflow_id: Option<&str>,
    event_type: Option<&str>,
    aggregate_id: Option<&str>,
) -> Result<(i32, CanonicalValue), CliFailure> {
    // A negative value outside the machine range is still negative, and the
    // application reports its own bounded-input message. A positive value
    // outside the range cannot be bound by SQLite at all; CPython raises an
    // uncaught OverflowError, so the native line reports the same failure
    // class rather than silently returning an empty page.
    let after_sequence = match i64::try_from(after_sequence.clone()) {
        Ok(value) => value,
        Err(_) if after_sequence.sign() == num_bigint::Sign::Minus => -1,
        Err(_) => return Err(CliFailure::LocalFailure("OverflowError")),
    };
    let limit = clamp_i64(limit);
    let page =
        application.list_events(after_sequence, limit, workflow_id, event_type, aggregate_id)?;
    Ok((
        EXIT_OK,
        CanonicalValue::object([
            ("command", CanonicalValue::text("events")),
            ("data", page.to_canonical_value()?),
            ("status", CanonicalValue::text("listed")),
        ]),
    ))
}

fn clamp_i64(value: &BigInt) -> i64 {
    match i64::try_from(value.clone()) {
        Ok(value) => value,
        Err(_) if value.sign() == num_bigint::Sign::Minus => i64::MIN,
        Err(_) => i64::MAX,
    }
}

fn run_exit_code(status: &str) -> i32 {
    if status == "waiting_for_approval" || status == "completed" {
        EXIT_OK
    } else {
        EXIT_FAILED_OUTCOME
    }
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

fn parse_parameter(raw: &str) -> Result<(String, CanonicalValue), CliFailure> {
    let Some((name, value)) = raw.split_once('=') else {
        return Err(CliFailure::Application(ApplicationError::application(
            "a parameter override must be written as name=value",
        )));
    };
    let name = kala_api::pytext::trim(name);
    if name.is_empty() {
        return Err(CliFailure::Application(ApplicationError::application(
            "a parameter override must be written as name=value",
        )));
    }
    let decoded = match json::loads(value) {
        Ok(decoded) => decoded,
        Err(json::LoadError::TooDeep) => {
            return Err(CliFailure::LocalFailure("RecursionError"));
        }
        Err(_) => CanonicalValue::text(value),
    };
    Ok((name.to_owned(), decoded))
}

fn parse_parameters(
    overrides: &[String],
    raw: Option<&str>,
) -> Result<Option<CanonicalValue>, CliFailure> {
    if raw.is_none() && overrides.is_empty() {
        return Ok(None);
    }
    let mut parameters = kala_json::CanonicalObject::new();
    if let Some(raw) = raw {
        let decoded = match json::loads(raw) {
            Ok(decoded) => decoded,
            Err(json::LoadError::TooDeep) => {
                return Err(CliFailure::LocalFailure("RecursionError"));
            }
            Err(_) => {
                return Err(CliFailure::Application(ApplicationError::application(
                    "--parameters-json must be a JSON object",
                )));
            }
        };
        let CanonicalValue::Object(entries) = &decoded else {
            return Err(CliFailure::Application(ApplicationError::application(
                "--parameters-json must be a JSON object",
            )));
        };
        parameters.extend(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    for item in overrides {
        let (name, value) = parse_parameter(item)?;
        parameters.insert(name.into(), value);
    }
    if parameters.len() > MAX_PARAMETERS_ENTRIES {
        return Err(CliFailure::Application(ApplicationError::application(
            "too many parameter overrides were supplied",
        )));
    }
    let value = CanonicalValue::Object(parameters);
    if json::default_dumps(&value).len() > MAX_PARAMETER_BYTES {
        return Err(CliFailure::Application(ApplicationError::application(
            "the parameter overrides exceed the bounded size",
        )));
    }
    Ok(Some(value))
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

fn path_value(path: &Path) -> CanonicalValue {
    CanonicalValue::text(path.to_string_lossy().into_owned())
}

fn emit(stream: &mut dyn Write, payload: &CanonicalValue) -> std::io::Result<()> {
    stream.write_all(json::pretty(payload).as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn emit_error(stream: &mut dyn Write, code: &str, message: &str) -> std::io::Result<()> {
    let truncated: String = message.chars().take(512).collect();
    let payload = CanonicalValue::object([(
        "error",
        CanonicalValue::object([
            ("code", CanonicalValue::text(code)),
            ("message", CanonicalValue::text(truncated)),
        ]),
    )]);
    emit(stream, &payload)
}
