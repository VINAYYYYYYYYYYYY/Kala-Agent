//! The local Kala API server.
//!
//! The binary is the native counterpart of the `serve` command of the Python
//! command line adapter: it opens the same local data directory, mints the
//! same per-launch bearer token, writes it to that directory, and serves the
//! same local API on the loopback interface with the same Host guard and the
//! same token gate every other launch of this API carries.
//!
//! The launch contract is the one of the Python entry point:
//!
//! * `--data-dir` names the local data directory and overrides
//!   `KALA_AGENT_DATA_DIR` for this invocation only;
//! * `--host` names the loopback address to bind and refuses anything that is
//!   not a loopback interface, before a socket exists;
//! * `--port` names the bounded TCP port to bind;
//! * the settings of the launch come from the seven environment variables the
//!   configuration reads, and from nothing else;
//! * the token is written to `<data-dir>/api-token` with owner-only
//!   permissions and is never printed, never logged and never returned;
//! * the summary of the launch is one JSON document on stdout, printed when
//!   the server stops, and every failure is one JSON document on stderr.
//!
//! Exit codes: `0` the server stopped, `1` the launch could not be completed,
//! `2` a usage error.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;

use kala_api::boundary::{is_loopback_host, issue_api_token};
use kala_api::config::{Settings, settings_from_env};
use kala_api_server::{LaunchSummary, launch};
use kala_application::KalaApplication;

/// The name of the program.
const PROGRAM_NAME: &str = "kala-api-server";

/// The exit code of a launch that completed.
const EXIT_OK: u8 = 0;

/// The exit code of a launch that could not be completed.
const EXIT_ERROR: u8 = 1;

/// The exit code of a usage error.
const EXIT_USAGE: u8 = 2;

/// The default address the server binds.
const SERVE_HOST: &str = "127.0.0.1";

/// The default port the server binds.
const SERVE_PORT: u16 = 8000;

/// The lowest port the server binds.
const MIN_SERVE_PORT: u16 = 1;

/// The highest port the server binds.
const MAX_SERVE_PORT: u16 = 65_535;

/// The environment variable that names the local data directory.
const DATA_DIR_ENVIRONMENT_KEY: &str = "KALA_AGENT_DATA_DIR";

/// The message of a refused address.
const NON_LOOPBACK_HOST_MESSAGE: &str =
    "serve binds the loopback interface only; a routable or wildcard address is refused";

/// The usage of the program.
const USAGE: &str = "usage: kala-api-server [-h] [--data-dir DATA_DIR] [--host HOST] [--port PORT]";

/// The arguments of one launch.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Arguments {
    /// The local data directory, if the caller named one.
    data_dir: Option<String>,
    /// The address to bind.
    host: String,
    /// The port to bind.
    port: u32,
}

/// The arguments the program was called with.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Invocation {
    /// The arguments of the launch.
    Run(Arguments),
    /// The caller asked for the usage.
    Help,
    /// The arguments are not usable.
    Usage(String),
}

/// Read the arguments of the launch.
fn parse(arguments: &[String]) -> Invocation {
    let mut parsed = Arguments {
        data_dir: None,
        host: SERVE_HOST.to_owned(),
        port: u32::from(SERVE_PORT),
    };
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        match argument {
            "-h" | "--help" => return Invocation::Help,
            "--data-dir" | "--host" | "--port" => {
                let Some(value) = arguments.get(index + 1) else {
                    return Invocation::Usage(format!(
                        "argument {argument}: expected one argument"
                    ));
                };
                match argument {
                    "--data-dir" => parsed.data_dir = Some(value.clone()),
                    "--host" => parsed.host = value.clone(),
                    _ => match value.parse::<u32>() {
                        Ok(port) => parsed.port = port,
                        Err(_) => {
                            return Invocation::Usage(format!(
                                "argument --port: invalid int value: '{value}'"
                            ));
                        }
                    },
                }
                index += 2;
            }
            _ => {
                if let Some(value) = argument.strip_prefix("--data-dir=") {
                    parsed.data_dir = Some(value.to_owned());
                } else if let Some(value) = argument.strip_prefix("--host=") {
                    parsed.host = value.to_owned();
                } else if let Some(value) = argument.strip_prefix("--port=") {
                    match value.parse::<u32>() {
                        Ok(port) => parsed.port = port,
                        Err(_) => {
                            return Invocation::Usage(format!(
                                "argument --port: invalid int value: '{value}'"
                            ));
                        }
                    }
                } else {
                    return Invocation::Usage(format!("unrecognized arguments: {argument}"));
                }
                index += 1;
            }
        }
    }
    Invocation::Run(parsed)
}

/// The settings of the launch.
///
/// The environment of the process is read for the seven variables the
/// configuration declares, and the data directory the caller named overrides
/// the environment for this invocation only.
fn settings_for(data_dir: Option<&str>) -> Result<Settings, String> {
    let mut environ: BTreeMap<String, String> = std::env::vars().collect();
    if let Some(data_dir) = data_dir {
        environ.insert(DATA_DIR_ENVIRONMENT_KEY.to_owned(), data_dir.to_owned());
    }
    settings_from_env(&environ).map_err(|errors| {
        let fields = errors
            .iter()
            .map(|error| format!("{}:{}", error.location, error.kind))
            .collect::<Vec<String>>()
            .join(",");
        if fields.is_empty() {
            "the local configuration is invalid".to_owned()
        } else {
            format!("the local configuration is invalid: {fields}")
        }
    })
}

/// The address the server binds.
fn serve_target(host: &str, port: u32) -> Result<(String, u16), String> {
    let resolved = host.trim();
    if !is_loopback_host(resolved) {
        return Err(NON_LOOPBACK_HOST_MESSAGE.to_owned());
    }
    if !(u32::from(MIN_SERVE_PORT)..=u32::from(MAX_SERVE_PORT)).contains(&port) {
        return Err(format!(
            "the port must be between {MIN_SERVE_PORT} and {MAX_SERVE_PORT}"
        ));
    }
    Ok((
        resolved.to_owned(),
        u16::try_from(port).expect("the port is inside the accepted range"),
    ))
}

/// The URL of the launch.
fn serve_url(host: &str, port: u16) -> String {
    let authority = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    format!("http://{authority}:{port}")
}

/// One failure of the launch.
struct LaunchError {
    /// The machine readable code of the failure.
    code: &'static str,
    /// The message of the failure.
    message: String,
}

impl LaunchError {
    /// Build one failure.
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Run the launch.
fn run(arguments: &Arguments) -> Result<(), LaunchError> {
    let (host, port) = serve_target(&arguments.host, arguments.port)
        .map_err(|message| LaunchError::new("invalid_address", message))?;
    let settings = settings_for(arguments.data_dir.as_deref())
        .map_err(|message| LaunchError::new("invalid_configuration", message))?;
    let data_dir = settings.data_dir.clone();

    let application = Arc::new(
        KalaApplication::open(settings)
            .map_err(|error| LaunchError::new("local_failure", error.message().to_owned()))?,
    );

    // The token is minted and written before the socket exists, so the
    // operator can read it while the server runs.
    let token =
        issue_api_token(&data_dir).map_err(|message| LaunchError::new("local_failure", message))?;

    // The router, the request boundary, the socket and the shutdown are the
    // library launch, which the `kala-agent serve` command shares.
    let LaunchSummary {
        host: bound_host,
        port: bound_port,
        token_path,
    } = launch(Arc::clone(&application), &host, port, &token)
        .map_err(|error| LaunchError::new(error.code(), error.message()))?
        .summary;

    // The composition root is released by the launch when it stops.

    let summary = serde_json::json!({
        "command": "serve",
        "data": {
            "api_token_path": token_path.to_string_lossy(),
            "authentication": "bearer",
            "host": bound_host,
            "loopback_only": true,
            "port": bound_port,
            "url": serve_url(&bound_host, bound_port),
        },
        "status": "stopped",
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&summary)
            .map_err(|error| LaunchError::new("local_failure", error.to_string()))?
    );
    Ok(())
}

/// The entry point of the program.
fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match parse(&arguments) {
        Invocation::Help => {
            println!("{USAGE}");
            ExitCode::from(EXIT_OK)
        }
        Invocation::Usage(message) => {
            eprintln!("{USAGE}");
            eprintln!("{PROGRAM_NAME}: error: {message}");
            ExitCode::from(EXIT_USAGE)
        }
        Invocation::Run(arguments) => match run(&arguments) {
            Ok(()) => ExitCode::from(EXIT_OK),
            Err(error) => {
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "error": {
                            "code": error.code,
                            "message": error.message.chars().take(512).collect::<String>(),
                        }
                    }))
                    .unwrap_or_else(|_| "{\"error\": {\"code\": \"local_failure\"}}".to_owned())
                );
                ExitCode::from(EXIT_ERROR)
            }
        },
    }
}
