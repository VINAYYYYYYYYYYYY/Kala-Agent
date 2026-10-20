//! The `kala-studio` command line adapter.
//!
//! Flags, loopback validation, exit codes, data/static resolution and the
//! sorted JSON launch banner mirror `python -m kala_agent.studio`. Readiness
//! is a health request from the desktop shell; graceful shutdown handles
//! SIGTERM and Ctrl-C so the Electron 15 s / 3 s termination contract holds.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;

use crate::boundary::{
    NOT_LOOPBACK_MESSAGE, STUDIO_TOKEN_ENV_VAR, default_studio_data_dir, is_loopback_host,
};
use crate::geometry_backend::{GeometryBackend, PythonWorkerBackend};
use crate::json::Json;
use crate::pytext;
use crate::server::{StudioConfig, StudioService};
use crate::static_files::{STUDIO_DIST_ENV_VAR, expand_path, resolve_static_root};
use crate::worker::{GeometryWorker, WorkerCommand, development_import_roots};

pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 8765;
pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

/// Run the standalone Kala Studio HTTP service on a loopback interface.
#[derive(Parser, Debug)]
#[command(
    name = "kala-studio",
    about = "Run the Kala Studio standalone HTTP service on a loopback interface.",
    disable_version_flag = true
)]
struct Arguments {
    /// Loopback address; refused if not 127.0.0.1, ::1, or localhost.
    #[arg(long, default_value = DEFAULT_HOST)]
    host: String,
    /// TCP port to bind, 1 to 65535.
    #[arg(long, default_value_t = i64::from(DEFAULT_PORT))]
    port: i64,
    /// Studio data directory; defaults to the standard data directory or temp.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Override the static UI directory.
    #[arg(long)]
    static_dist: Option<PathBuf>,
}

/// A validated bind target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeTarget {
    pub host: String,
    pub port: u16,
}

impl ServeTarget {
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

/// Resolve and validate a loopback host/port pair.
///
/// # Errors
///
/// Returns the Python `ValueError` text for a non-loopback host or an
/// out-of-range port.
pub fn build_target(host: Option<&str>, port: Option<i64>) -> Result<ServeTarget, String> {
    let resolved_host = host
        .map(|value| pytext::trim(value).to_owned())
        .unwrap_or_else(|| DEFAULT_HOST.to_owned());
    if !is_loopback_host(&resolved_host) {
        return Err(NOT_LOOPBACK_MESSAGE.to_owned());
    }
    let resolved_port = port.unwrap_or(i64::from(DEFAULT_PORT));
    if !(1..=65_535).contains(&resolved_port) {
        return Err("port must be between 1 and 65535".to_owned());
    }
    Ok(ServeTarget {
        host: resolved_host,
        port: resolved_port as u16,
    })
}

/// CLI entry point returning the process exit code.
pub fn main() -> i32 {
    let arguments = match Arguments::try_parse() {
        Ok(arguments) => arguments,
        Err(error) => {
            let _ = error.print();
            return if error.use_stderr() {
                EXIT_USAGE
            } else {
                EXIT_OK
            };
        }
    };
    let target = match build_target(Some(&arguments.host), Some(arguments.port)) {
        Ok(target) => target,
        Err(error) => {
            eprintln!("{NOT_LOOPBACK_MESSAGE}: {error}");
            return EXIT_USAGE;
        }
    };
    let environ: HashMap<String, String> = crate::boundary::process_environ().into_iter().collect();
    let environ_view: BTreeMap<String, String> = environ
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    // The STEP import ceiling is resolved once, before anything is started:
    // the front door and the worker it launches must always agree, and an
    // unusable value refuses startup exactly like the Python worker.
    let import_max_bytes = match crate::step_limit::resolve_step_import_max_bytes(&environ_view) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return EXIT_USAGE;
        }
    };
    let data_dir = match &arguments.data_dir {
        Some(path) => match expand_path(&path.to_string_lossy()) {
            Some(path) => path,
            None => {
                eprintln!("the studio data directory could not be resolved");
                return EXIT_USAGE;
            }
        },
        None => default_studio_data_dir(&environ_view),
    };
    let static_root = match &arguments.static_dist {
        Some(path) => expand_path(&path.to_string_lossy()),
        None => {
            let executable_dir = std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
            resolve_static_root(&environ, executable_dir.as_deref())
        }
    };
    emit_banner(&target, &data_dir);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("the studio runtime could not be initialised");
            return EXIT_ERROR;
        }
    };
    runtime.block_on(async move {
        // ADR-0010 phase 2: the native geometry backend is the default. The
        // supervised Python worker is started **only** when the dev opt-in
        // `KALA_STUDIO_GEOMETRY_WORKER` names one, so a default launch has no
        // Python process anywhere in the picture.
        let worker = start_geometry_worker(&data_dir, import_max_bytes).await;
        let backend: Option<Arc<dyn GeometryBackend>> = match worker.as_ref() {
            Some(worker) => match PythonWorkerBackend::new(Arc::clone(worker)) {
                Ok(backend) => Some(backend),
                Err(error) => {
                    eprintln!("{error}");
                    None
                }
            },
            None => match native_backend(&data_dir, import_max_bytes) {
                Ok(backend) => Some(backend),
                Err(error) => {
                    eprintln!("{error}");
                    None
                }
            },
        };
        let service = match StudioService::new(StudioConfig {
            data_dir: Some(data_dir.clone()),
            api_token: None,
            max_body_bytes: crate::server::MAX_REQUEST_BODY_BYTES,
            static_root,
            environ: None,
            import_max_bytes: Some(import_max_bytes),
            step_exporter: crate::geometry::default_step_exporter(),
            transport: None,
            geometry_backend: backend,
        }) {
            Ok(service) => service,
            Err(error) => {
                eprintln!("{error}");
                if let Some(worker) = worker.as_ref() {
                    worker.shutdown().await;
                }
                return EXIT_ERROR;
            }
        };
        let listener = match TcpListener::bind((target.host.as_str(), target.port)).await {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("the studio could not bind {}: {error}", target.url());
                if let Some(worker) = worker.as_ref() {
                    worker.shutdown().await;
                }
                return EXIT_ERROR;
            }
        };
        let result = service.serve(listener, shutdown_signal()).await;
        if let Some(worker) = worker.as_ref() {
            worker.shutdown().await;
        }
        match result {
            Ok(()) => EXIT_OK,
            Err(error) => {
                eprintln!("the studio stopped: {error}");
                EXIT_ERROR
            }
        }
    })
}

/// Build the native geometry backend.
///
/// The CAD provider connections see the same controlled environment the
/// supervised Python worker saw, so a FreeCAD probe reports the same facts on
/// both sides of the live differential.
#[cfg(feature = "native-geometry")]
fn native_backend(
    data_dir: &std::path::Path,
    import_max_bytes: usize,
) -> Result<Arc<dyn GeometryBackend>, String> {
    use crate::native_geometry::NativeGeometryBackend;

    let workspace = data_dir.join("geometry-worker");
    let _ = std::fs::create_dir_all(&workspace);
    let environ = provider_environment(&workspace);
    let limit = u64::try_from(import_max_bytes).unwrap_or(u64::MAX);
    NativeGeometryBackend::new(
        data_dir.to_path_buf(),
        environ,
        limit,
        crate::geometry::default_step_exporter(),
        data_dir.join("geometry-work"),
    )
    .map(|backend| backend as Arc<dyn GeometryBackend>)
}

/// The controlled environment the CAD provider subprocesses see.
#[cfg(feature = "native-geometry")]
fn provider_environment(workspace: &std::path::Path) -> BTreeMap<String, String> {
    match kala_process::ChildEnvironment::legacy_python_worker(workspace, &[]) {
        Ok(environment) => environment
            .entries()
            .iter()
            .map(|entry| (entry.key.clone(), entry.value.clone()))
            .collect(),
        Err(_) => crate::boundary::process_environ().into_iter().collect(),
    }
}

/// The refusal of a build without the native geometry backend.
#[cfg(not(feature = "native-geometry"))]
fn native_backend(
    _data_dir: &std::path::Path,
    _import_max_bytes: usize,
) -> Result<Arc<dyn GeometryBackend>, String> {
    Err("this studio build links no native geometry backend".to_owned())
}

/// Start the supervised geometry worker, when one is configured.
///
/// The worker is the **development fallback**: it is started only when the
/// `KALA_STUDIO_GEOMETRY_WORKER` environment variable names one, so the
/// default launch never starts a Python process.
///
/// The front door's resolved `data_dir` is handed to the worker as
/// `--data-dir`: the worker's journal and artifacts are written there, never
/// into the user's home directory and never into a shared temporary store.
async fn start_geometry_worker(
    data_dir: &std::path::Path,
    import_max_bytes: usize,
) -> Option<Arc<GeometryWorker>> {
    let environ: BTreeMap<String, String> =
        crate::boundary::process_environ().into_iter().collect();
    if !worker_opted_in(&environ) {
        return None;
    }
    let command = WorkerCommand::from_environment(&environ)?;
    let workspace = data_dir.join("geometry-worker");
    if let Err(error) = std::fs::create_dir_all(&workspace) {
        eprintln!("the geometry worker directory could not be created: {error}");
        return None;
    }
    let worker = GeometryWorker::new(
        command,
        data_dir.to_path_buf(),
        workspace,
        development_import_roots(),
        import_max_bytes,
    );
    worker.start().await;
    Some(worker)
}

/// Whether the operator asked for the supervised Python worker.
///
/// The variable names the worker command; an absent or blank value means the
/// native backend is used and no Python process is started.
fn worker_opted_in(environ: &BTreeMap<String, String>) -> bool {
    environ
        .get(crate::worker::WORKER_COMMAND_ENV_VAR)
        .is_some_and(|value| !value.trim().is_empty())
}

fn emit_banner(target: &ServeTarget, data_dir: &std::path::Path) {
    let document = Json::Object(vec![
        (
            "data_dir".to_owned(),
            Json::String(data_dir.display().to_string()),
        ),
        ("host".to_owned(), Json::String(target.host.clone())),
        ("loopback".to_owned(), Json::Bool(true)),
        ("port".to_owned(), Json::UInt(u64::from(target.port))),
        (
            "static_dist_environment_variable".to_owned(),
            Json::String(STUDIO_DIST_ENV_VAR.to_owned()),
        ),
        (
            "token_environment_variable".to_owned(),
            Json::String(STUDIO_TOKEN_ENV_VAR.to_owned()),
        ),
        ("url".to_owned(), Json::String(target.url())),
    ]);
    if let Ok(mut text) = document.to_pretty_sorted_ascii() {
        use std::io::Write;
        text.push('\n');
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = terminate.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_match_python_examples() {
        assert_eq!(
            build_target(None, None).unwrap(),
            ServeTarget {
                host: "127.0.0.1".to_owned(),
                port: 8765
            }
        );
        assert_eq!(
            build_target(Some("::1"), Some(9000)).unwrap().url(),
            "http://[::1]:9000"
        );
        assert_eq!(
            build_target(Some("localhost"), Some(8123)).unwrap().url(),
            "http://localhost:8123"
        );
        assert!(build_target(Some("0.0.0.0"), Some(8765)).is_err());
        assert!(build_target(Some("127.0.0.1"), Some(0)).is_err());
        assert!(build_target(Some("127.0.0.1"), Some(70_000)).is_err());
    }
}
