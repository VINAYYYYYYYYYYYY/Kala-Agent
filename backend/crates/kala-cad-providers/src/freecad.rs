//! The FreeCAD provider: a real `FreeCADCmd` subprocess (ADR-0007/0011).
//!
//! Core code never imports `FreeCAD` or the adapter distribution. This provider
//! writes the same small driver script as the Python provider, runs it in a
//! separate `FreeCADCmd` process with a bounded wall time and no shell, and
//! reads back the adapter's own result envelope. The driver uses the adapter's
//! official `worker.invoke_design` entry point: it lowers the shared
//! `ConstructionScript` with `kala_agent_freecad.cad.compile.compile_construction`,
//! builds the model in a FreeCAD document, exports STEP and FCStd, and reports
//! FreeCAD's measured properties. The adapter owns the export target, so a
//! placed part and a place-only assembly export the geometry they actually
//! describe.
//!
//! Bounds are measured by FreeCAD itself: after the build, the driver reopens
//! the exported FCStd and reads the exported shape's `BoundBox` and `Solids`.
//! If that reopen fails, bounds and the solid count are left unreported (never
//! guessed) and a warning says so.
//!
//! The driver source is embedded byte for byte from the Python provider
//! (`src/driver.py`); the differential test compares it with the live Python
//! module. The process itself is supervised by the accepted `kala-process`
//! crate: explicit absolute executable, explicit argv, no shell, no PATH
//! lookup in the supervisor, an explicit environment, a bounded wall deadline
//! and native tree cleanup.
//!
//! # Intentional differences from the Python provider
//!
//! * The child environment is built through `kala-process`, whose key syntax
//!   is `[A-Za-z_][A-Za-z0-9_]{0,63}`. Environment entries with other key
//!   shapes (for example exported shell functions) are skipped instead of
//!   making the spawn fail; non-UTF-8 names and values are skipped too.
//! * The adapter search path is discovered from the repository layout
//!   (`adapters/freecad/src`, `src`, and the repository's `.venv` site
//!   packages), or supplied explicitly. Python instead asks its own
//!   interpreter for `sysconfig`'s `purelib`; a deployment that needs a
//!   different path passes [`FreeCadProviderOptions::sys_path`].
//! * When the adapter ships per-ABI dependency directories
//!   (`site-packages-cp311` … `site-packages-cp314`), the provider probes
//!   `FreeCADCmd`'s own Python (`sys.implementation.cache_tag`) once per probe
//!   cache window and adds exactly the matching directory to `sys.path`. An
//!   unsupported Python is an unavailable provider whose reason names the
//!   interpreter version; it never loads another ABI's compiled extension.
//! * The response envelope is parsed with `kala-json`'s strict parser: a
//!   duplicate key or a non-finite number is refused as "did not report a
//!   result" rather than partially read. A successful FreeCAD build never
//!   emits either.
//! * The reviewer's stderr tail is the bounded capture retained by
//!   `kala-process` (up to 16 MiB) instead of an unbounded log file. Above
//!   that cap the earliest bytes are retained and the tail is unavailable.
//! * Probe output is decoded with replacement instead of raising
//!   `UnicodeDecodeError`, and a capture-limit exit is reported as a failed
//!   probe rather than a successful one.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use kala_contracts::{ConstructionScript, ContractModel, WireValue};
use kala_json::{
    CanonicalObject, CanonicalValue, JsonText, Limits, canonical_dumps, strict_json_loads,
};
use kala_process::{
    CancelToken, ChildEnvironment, EnvironmentEntry, ProcessEnd, ProcessError, ProcessLimits,
    ProcessOutcome, ProcessSpec, supervise,
};
use num_traits::ToPrimitive;
use regex::Regex;

use crate::base::{
    Availability, CadProvider, CadProviderBuildError, CadProviderConfigError, CadProviderError,
    CadProviderUnavailableError, CadValueError, MeasuredBounds, MeasuredProperties, ProviderBuild,
    truncate_chars,
};

/// The executable names searched on `PATH`, in order.
pub const FREECAD_EXECUTABLE_NAMES: [&str; 2] = ["FreeCADCmd", "freecadcmd"];
/// Directory-name prefix of the adapter's per-ABI Python dependency sets.
pub const SITE_PACKAGES_PREFIX: &str = "site-packages-";
/// The CPython ABIs the bundled FreeCAD adapter dependencies target.
///
/// Current FreeCAD releases ship Python 3.11, 3.12, 3.13 or 3.14. The packaged
/// adapter stages one `site-packages-<abi>` directory for each of these.
pub const SUPPORTED_FREECAD_PYTHON_ABIS: [&str; 4] = ["cp311", "cp312", "cp313", "cp314"];
/// The generated Python-identity probe's file name.
pub const PYTHON_PROBE_FILENAME: &str = "kala_freecad_python_probe.py";
/// Default bounded build wall time.
///
/// A 500-step design (a 2 000 x 2 000 x 5 mm plate with 249 through holes)
/// built in 1 009.5 s on this host (1 250 s under load), and the 202-step
/// 100-hole plate in 58.8 s. 1 800 s covers the measured whole-budget case
/// with 78 % margin when idle and 44 % margin under load.
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 1_800.0;
/// Smallest accepted build wall time.
pub const MIN_TIMEOUT_SECONDS: f64 = 5.0;
/// Largest accepted build wall time.
pub const MAX_TIMEOUT_SECONDS: f64 = 3_600.0;
/// Default probe cache duration.
pub const DEFAULT_PROBE_CACHE_SECONDS: f64 = 30.0;
/// Bound on the `FreeCADCmd --version` probe.
pub const VERSION_PROBE_TIMEOUT_SECONDS: f64 = 20.0;
/// Largest native artifact the provider reads back.
pub const NATIVE_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Largest result envelope the provider reads back.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// The generated driver script's file name.
pub const DRIVER_FILENAME: &str = "kala_freecad_provider_driver.py";
/// The request file's name.
pub const REQUEST_FILENAME: &str = "kala-freecad-request.json";
/// The response file's name.
pub const RESPONSE_FILENAME: &str = "kala-freecad-response.json";
/// Environment variable carrying the adapter search path to the driver.
pub const SYS_PATH_ENV_VAR: &str = "KALA_FREECAD_SYS_PATH";
/// Environment variable carrying the request path to the driver.
pub const REQUEST_ENV_VAR: &str = "KALA_FREECAD_REQUEST";
/// Environment variable carrying the workspace to the driver.
pub const WORKSPACE_ENV_VAR: &str = "KALA_FREECAD_WORKSPACE";
/// Environment variable carrying the response path to the driver.
pub const OUTPUT_ENV_VAR: &str = "KALA_FREECAD_OUTPUT";
/// Environment variable carrying the STEP size bound to the driver.
pub const MAX_STEP_ENV_VAR: &str = "KALA_FREECAD_MAX_STEP_BYTES";
/// Environment variable carrying the native artifact bound to the driver.
pub const MIN_ARTIFACT_BYTES_ENV_VAR: &str = "KALA_FREECAD_MAX_ARTIFACT_BYTES";
/// The `KALA_STEP_IMPORT_MAX_BYTES` operator setting the Python provider uses.
pub const STEP_IMPORT_MAX_BYTES_ENV_VAR: &str = "KALA_STEP_IMPORT_MAX_BYTES";
/// The exact unavailable reason when no `FreeCADCmd` is on `PATH`.
pub const UNAVAILABLE_REASON: &str =
    "FreeCADCmd was not found on PATH; install FreeCAD 1.1 or later to enable the FreeCAD provider";

/// The generated driver, byte for byte the Python provider's `_DRIVER_SOURCE`.
pub const DRIVER_SOURCE: &str = include_str!("driver.py");

/// The default STEP import ceiling, matching `importer.DEFAULT_MAX_STEP_BYTES`.
pub const DEFAULT_MAX_STEP_BYTES: usize = 64 * 1024 * 1024;
/// The smallest accepted STEP import ceiling.
pub const MIN_STEP_IMPORT_BYTES: usize = 1;
/// The largest accepted STEP import ceiling.
pub const MAX_STEP_IMPORT_BYTES: usize = 512 * 1024 * 1024;

/// How the bundled adapter's per-ABI dependency directories matched FreeCAD's
/// Python.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundledPythonSelection {
    /// The adapter ships no `site-packages-<abi>` directory (a development
    /// checkout); only its source directory is used.
    Unbundled,
    /// Exactly one matching dependency directory exists.
    Matched {
        /// The ABI tag of the selected directory.
        abi: String,
        /// The absolute `site-packages-<abi>` directory.
        directory: PathBuf,
    },
    /// Dependency directories exist, but none for the detected ABI.
    Unsupported {
        /// The ABI FreeCADCmd reported.
        detected: String,
        /// The bundled ABIs, sorted.
        supported: Vec<String>,
    },
}

/// List the per-ABI dependency directories bundled beside an adapter source
/// directory (`adapters/freecad/src`).
///
/// Only immediate `site-packages-cpNNN` directories are considered. An absent
/// or unreadable parent yields an empty list.
pub fn bundled_python_abis(adapter_src: &Path) -> Vec<String> {
    let Some(parent) = adapter_src.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut abis: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let abi = name.strip_prefix(SITE_PACKAGES_PREFIX)?;
            is_python_abi_tag(abi).then(|| abi.to_owned())
        })
        .collect();
    abis.sort();
    abis
}

/// Select the single bundled dependency directory for a detected ABI.
///
/// This is the runtime half of the ABI matrix: the provider probes
/// `FreeCADCmd`'s Python, derives the ABI tag and asks this function which
/// directory (if any) matches. It never falls back to another ABI's compiled
/// extension.
pub fn select_bundled_python_abi(adapter_src: &Path, detected_abi: &str) -> BundledPythonSelection {
    let supported = bundled_python_abis(adapter_src);
    if supported.is_empty() {
        return BundledPythonSelection::Unbundled;
    }
    if supported.iter().any(|abi| abi == detected_abi) {
        let Some(parent) = adapter_src.parent() else {
            return BundledPythonSelection::Unsupported {
                detected: detected_abi.to_owned(),
                supported,
            };
        };
        return BundledPythonSelection::Matched {
            abi: detected_abi.to_owned(),
            directory: parent.join(format!("{SITE_PACKAGES_PREFIX}{detected_abi}")),
        };
    }
    BundledPythonSelection::Unsupported {
        detected: detected_abi.to_owned(),
        supported,
    }
}

fn is_python_abi_tag(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("cp") else {
        return false;
    };
    let digits = rest.strip_suffix('t').unwrap_or(rest);
    !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
}

fn is_site_packages_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(SITE_PACKAGES_PREFIX))
        .is_some_and(is_python_abi_tag)
}

/// The result of the optional `FreeCADCmd` Python probe.
enum BundledProbe {
    /// No per-ABI directories are bundled, or the caller supplied one.
    None,
    /// The matching `site-packages-<abi>` directory.
    Selected(PathBuf),
    /// The probe failed or the ABI is unsupported; the reason is user-facing.
    Unavailable(String),
}

/// Keys whose names look credential-shaped are removed before the child runs.
const SENSITIVE_ENV_MARKERS: [&str; 5] = ["TOKEN", "API_KEY", "SECRET", "PASSWORD", "CREDENTIAL"];

const BOOTSTRAP_TIMEOUT_SECONDS: f64 = 5.0;
const TERMINATION_TIMEOUT_SECONDS: f64 = 5.0;
const PYTHON_PROBE_MARKER: &str = "KALA_FREECAD_PYTHON=";
const PYTHON_PROBE_SOURCE: &str = r#"import json, sys
info = {
    "version": sys.version.split()[0],
    "cacheTag": sys.implementation.cache_tag,
}
sys.stdout.write("KALA_FREECAD_PYTHON=" + json.dumps(info) + "\n")
"#;

/// Constructor options for [`FreeCadProvider`].
#[derive(Clone, Debug, Default)]
pub struct FreeCadProviderOptions {
    /// An explicit executable path. When absent, `FreeCADCmd` and
    /// `freecadcmd` are searched on the provider environment's `PATH`.
    pub executable: Option<PathBuf>,
    /// The bounded build wall time in seconds (5 to 600).
    pub timeout_seconds: Option<f64>,
    /// The STEP artifact ceiling. When absent, it is resolved from
    /// `KALA_STEP_IMPORT_MAX_BYTES`, then the 64 MiB default.
    pub max_step_bytes: Option<usize>,
    /// The probe cache duration in seconds.
    pub probe_cache_seconds: Option<f64>,
    /// The environment snapshot the child receives, after credential
    /// scrubbing. When absent, the process environment is snapshotted.
    pub environ: Option<BTreeMap<String, String>>,
    /// The adapter search path handed to the driver. When absent, the
    /// repository layout is discovered.
    pub sys_path: Option<Vec<PathBuf>>,
}

/// Build a construction script in a real, out-of-process FreeCAD.
#[derive(Clone, Debug)]
pub struct FreeCadProvider {
    executable: Option<PathBuf>,
    timeout_seconds: f64,
    max_step_bytes: usize,
    probe_cache_seconds: f64,
    environ: BTreeMap<String, String>,
    sys_path: Option<Vec<PathBuf>>,
    probe_cached: Option<Availability>,
    probe_at: Option<Instant>,
    abi_site_packages: Option<PathBuf>,
}

impl FreeCadProvider {
    /// Validate the options and build a provider.
    pub fn new(options: FreeCadProviderOptions) -> Result<Self, CadProviderConfigError> {
        let timeout_seconds = options.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        if !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&timeout_seconds) {
            return Err(CadProviderConfigError::new(format!(
                "the FreeCAD build timeout must be between {} and {} seconds",
                format_g(MIN_TIMEOUT_SECONDS),
                format_g(MAX_TIMEOUT_SECONDS)
            )));
        }
        let probe_cache_seconds = options
            .probe_cache_seconds
            .unwrap_or(DEFAULT_PROBE_CACHE_SECONDS);
        if probe_cache_seconds < 0.0 {
            return Err(CadProviderConfigError::new(
                "the probe cache duration must be non-negative",
            ));
        }
        let environ = options
            .environ
            .unwrap_or_else(crate::base::snapshot_environment);
        let max_step_bytes = match options.max_step_bytes {
            Some(value) => validate_step_import_max_bytes(value)?,
            None => resolve_step_import_max_bytes(&environ)?,
        };
        Ok(Self {
            executable: options.executable,
            timeout_seconds,
            max_step_bytes,
            probe_cache_seconds,
            environ,
            sys_path: options.sys_path,
            probe_cached: None,
            probe_at: None,
            abi_site_packages: None,
        })
    }

    /// A provider with the Python defaults.
    pub fn with_defaults() -> Result<Self, CadProviderConfigError> {
        Self::new(FreeCadProviderOptions::default())
    }

    /// The configured STEP artifact ceiling.
    pub const fn max_step_bytes(&self) -> usize {
        self.max_step_bytes
    }

    /// The configured build wall time in seconds.
    pub const fn timeout_seconds(&self) -> f64 {
        self.timeout_seconds
    }

    /// The environment snapshot the child receives (before credential
    /// scrubbing), for diagnostics.
    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environ
    }

    /// Return whether a usable `FreeCADCmd` is present on this host.
    ///
    /// When the adapter bundles per-ABI dependency directories, this also
    /// probes `FreeCADCmd`'s own Python and caches the one matching
    /// `site-packages-<abi>` directory. An unsupported Python is reported as
    /// an unavailable provider whose reason names the interpreter version.
    pub fn probe(&mut self) -> Availability {
        let now = Instant::now();
        if let (Some(cached), Some(at)) = (&self.probe_cached, self.probe_at) {
            if now.duration_since(at).as_secs_f64() < self.probe_cache_seconds {
                return cached.clone();
            }
        }
        let (availability, site_packages) = self.probe_uncached();
        self.probe_cached = Some(availability.clone());
        self.probe_at = Some(now);
        self.abi_site_packages = site_packages;
        availability
    }

    fn probe_uncached(&self) -> (Availability, Option<PathBuf>) {
        let Some(executable) = self.resolve_executable() else {
            return (
                Availability::new(false, None::<String>, Some(UNAVAILABLE_REASON)),
                None,
            );
        };
        let environment = match self.probe_environment() {
            Ok(environment) => environment,
            Err(error) => {
                return (
                    Availability::new(
                        false,
                        None::<String>,
                        Some(format!(
                            "the FreeCADCmd version probe failed: {}",
                            environment_error_name(&error)
                        )),
                    ),
                    None,
                );
            }
        };
        let outcome = run_bounded(
            &executable,
            &[OsString::from("--version")],
            &current_directory(),
            environment.clone(),
            VERSION_PROBE_TIMEOUT_SECONDS,
        );
        match outcome {
            Err(error) => (
                Availability::new(
                    false,
                    None::<String>,
                    Some(format!(
                        "the FreeCADCmd version probe failed: {}",
                        probe_error_name(&error, &executable)
                    )),
                ),
                None,
            ),
            Ok(outcome) => match outcome.end {
                ProcessEnd::TimedOut(_) => (
                    Availability::new(
                        false,
                        None::<String>,
                        Some(format!(
                            "the FreeCADCmd version probe at {} did not finish within {} seconds",
                            python_repr(&executable.to_string_lossy()),
                            format_g(VERSION_PROBE_TIMEOUT_SECONDS)
                        )),
                    ),
                    None,
                ),
                ProcessEnd::Exited(_) => {
                    let output = format!(
                        "{}\n{}",
                        outcome.stdout.to_string_lossy(),
                        outcome.stderr.to_string_lossy()
                    );
                    match version_pattern().captures(&output) {
                        Some(captures) => {
                            let availability = Availability::new(
                                true,
                                captures.get(1).map(|value| value.as_str().to_owned()),
                                None::<String>,
                            );
                            match self.bundled_abi_site_packages(&executable, &environment) {
                                BundledProbe::None => (availability, None),
                                BundledProbe::Selected(directory) => {
                                    (availability, Some(directory))
                                }
                                BundledProbe::Unavailable(reason) => {
                                    (Availability::new(false, None::<String>, Some(reason)), None)
                                }
                            }
                        }
                        None => {
                            let detail = truncate_chars(&split_and_join(&output), 200);
                            let reason = if detail.is_empty() {
                                "FreeCADCmd did not report a FreeCAD version".to_owned()
                            } else {
                                format!("FreeCADCmd did not report a FreeCAD version: {detail}")
                            };
                            (Availability::new(false, None::<String>, Some(reason)), None)
                        }
                    }
                }
                _ => (
                    Availability::new(
                        false,
                        None::<String>,
                        Some("the FreeCADCmd version probe failed: OSError".to_owned()),
                    ),
                    None,
                ),
            },
        }
    }

    /// Probe FreeCADCmd's Python and select its bundled dependency directory.
    ///
    /// This runs only when the adapter actually ships per-ABI directories. A
    /// development checkout (no `site-packages-cpNNN`) keeps the old, single
    /// `sys.path` behaviour and does not spawn an extra process.
    fn bundled_abi_site_packages(
        &self,
        executable: &Path,
        environment: &ChildEnvironment,
    ) -> BundledProbe {
        let candidates: Vec<PathBuf> = match &self.sys_path {
            Some(paths) => paths.clone(),
            None => discover_driver_sys_path(),
        };
        // A caller that already supplies an ABI directory is authoritative.
        if self
            .sys_path
            .as_ref()
            .is_some_and(|paths| paths.iter().any(|path| is_site_packages_dir(path)))
        {
            return BundledProbe::None;
        }
        let Some(adapter_src) = candidates
            .into_iter()
            .find(|path| !bundled_python_abis(path).is_empty())
        else {
            return BundledProbe::None;
        };
        let temporary = match tempfile::Builder::new()
            .prefix("kala-freecad-python-probe-")
            .tempdir()
        {
            Ok(directory) => directory,
            Err(error) => {
                return BundledProbe::Unavailable(format!(
                    "the FreeCADCmd Python probe could not create a temporary directory: {error}"
                ));
            }
        };
        let script = temporary.path().join(PYTHON_PROBE_FILENAME);
        if let Err(error) = std::fs::write(&script, PYTHON_PROBE_SOURCE) {
            return BundledProbe::Unavailable(format!(
                "the FreeCADCmd Python probe could not be written: {error}"
            ));
        }
        let outcome = run_bounded(
            executable,
            &[script.into_os_string()],
            temporary.path(),
            environment.clone(),
            VERSION_PROBE_TIMEOUT_SECONDS,
        );
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                return BundledProbe::Unavailable(format!(
                    "the FreeCADCmd Python probe failed: {}",
                    probe_error_name(&error, executable)
                ));
            }
        };
        if let ProcessEnd::TimedOut(_) = outcome.end {
            return BundledProbe::Unavailable(format!(
                "the FreeCADCmd Python probe at {} did not finish within {} seconds",
                python_repr(&executable.to_string_lossy()),
                format_g(VERSION_PROBE_TIMEOUT_SECONDS)
            ));
        }
        let output = format!(
            "{}\n{}",
            outcome.stdout.to_string_lossy(),
            outcome.stderr.to_string_lossy()
        );
        let Some(line) = output
            .lines()
            .find(|line| line.starts_with(PYTHON_PROBE_MARKER))
        else {
            let detail = truncate_chars(&split_and_join(&output), 200);
            let exit = outcome
                .exit
                .map(|status| status.python_exit_code())
                .unwrap_or(0);
            return BundledProbe::Unavailable(if detail.is_empty() {
                format!("FreeCADCmd did not report its Python version (exit {exit})")
            } else {
                format!("FreeCADCmd did not report its Python version (exit {exit}): {detail}")
            });
        };
        let payload = &line[PYTHON_PROBE_MARKER.len()..];
        let Ok(CanonicalValue::Object(ref info)) = strict_json_loads(payload.as_bytes()) else {
            return BundledProbe::Unavailable(
                "FreeCADCmd reported an unreadable Python identity".to_owned(),
            );
        };
        let full_version = canonical_text(info, "version");
        let Some(abi) = canonical_text(info, "cacheTag")
            .as_deref()
            .and_then(abi_from_cache_tag)
        else {
            return BundledProbe::Unavailable(
                "FreeCADCmd reported no usable Python ABI tag".to_owned(),
            );
        };
        match select_bundled_python_abi(&adapter_src, &abi) {
            BundledPythonSelection::Unbundled => BundledProbe::None,
            BundledPythonSelection::Matched { directory, .. } => BundledProbe::Selected(directory),
            BundledPythonSelection::Unsupported {
                detected,
                supported,
            } => BundledProbe::Unavailable(format!(
                "FreeCADCmd runs Python {} ({detected}); the bundled adapter dependencies \
                     support {} only. Install a FreeCAD build with a supported Python.",
                full_version.as_deref().unwrap_or("an unknown version"),
                supported.join(", ")
            )),
        }
    }

    /// Build `script` in FreeCAD and return its STEP, FCStd, and numbers.
    pub fn build(
        &mut self,
        script: &ConstructionScript,
    ) -> Result<ProviderBuild, CadProviderError> {
        self.build_cancellable(script, &CancelToken::new())
    }

    /// Build `script`, killing the `FreeCADCmd` process group when `cancel`
    /// is set.
    ///
    /// The kill is the supervised process tree's own: `kala_process` polls the
    /// token on every slice and terminates the whole group, so a cancelled
    /// 17-minute build does not keep running in the background.
    pub fn build_cancellable(
        &mut self,
        script: &ConstructionScript,
        cancel: &CancelToken,
    ) -> Result<ProviderBuild, CadProviderError> {
        if cancel.is_cancelled() {
            return Err(CadProviderError::Cancelled(
                "the FreeCAD build was cancelled before it started".to_owned(),
            ));
        }
        let availability = self.probe();
        if !availability.is_available() {
            return Err(CadProviderUnavailableError::new(
                self.name(),
                format!("the {} CAD provider is not available", self.name()),
                availability
                    .reason()
                    .map(str::to_owned)
                    .or_else(|| Some(UNAVAILABLE_REASON.to_owned())),
                Vec::new(),
            )
            .into());
        }
        let Some(executable) = self.resolve_executable() else {
            return Err(CadProviderUnavailableError::new(
                self.name(),
                format!("the {} CAD provider is not available", self.name()),
                Some(UNAVAILABLE_REASON),
                Vec::new(),
            )
            .into());
        };
        let temporary = tempfile::Builder::new()
            .prefix("kala-freecad-provider-")
            .tempdir()
            .map_err(|error| {
                CadProviderBuildError::new(format!(
                    "the FreeCAD provider could not create a temporary directory: {error}"
                ))
            })?;
        let root = temporary.path();
        let workspace = root.join("work");
        std::fs::create_dir_all(&workspace).map_err(|error| {
            CadProviderBuildError::new(format!(
                "the FreeCAD provider could not create its workspace: {error}"
            ))
        })?;
        let request_path = root.join(REQUEST_FILENAME);
        let response_path = root.join(RESPONSE_FILENAME);
        let driver_path = root.join(DRIVER_FILENAME);
        let request = canonical_request_json(script)?;
        std::fs::write(&request_path, request.as_bytes()).map_err(|error| {
            CadProviderBuildError::new(format!(
                "the FreeCAD provider could not write its request: {error}"
            ))
        })?;
        std::fs::write(&driver_path, DRIVER_SOURCE).map_err(|error| {
            CadProviderBuildError::new(format!(
                "the FreeCAD provider could not write its driver: {error}"
            ))
        })?;
        let environment = self
            .build_environment(&request_path, &workspace, &response_path)
            .map_err(|error| {
                CadProviderBuildError::new(format!(
                    "the FreeCADCmd subprocess could not start: {}",
                    environment_error_name(&error)
                ))
            })?;
        let outcome = run_bounded_with_cancel(
            &executable,
            &[driver_path.clone().into_os_string()],
            root,
            environment,
            self.timeout_seconds,
            cancel,
        )
        .map_err(|error| {
            CadProviderBuildError::new(format!(
                "the FreeCADCmd subprocess could not start: {}",
                probe_error_name(&error, &executable)
            ))
        })?;
        if let ProcessEnd::Cancelled(_) = outcome.end {
            return Err(CadProviderError::Cancelled(
                "the FreeCAD build was cancelled; the FreeCADCmd process group was stopped"
                    .to_owned(),
            ));
        }
        if let ProcessEnd::TimedOut(_) = outcome.end {
            return Err(CadProviderBuildError::new(format!(
                "FreeCAD did not finish within {} seconds",
                format_g(self.timeout_seconds)
            ))
            .into());
        }
        let payload = read_response(&response_path);
        let Some(payload) = payload else {
            let detail = stderr_tail(outcome.stderr.as_bytes(), 400);
            let message = format!(
                "FreeCAD did not report a result (exit {}){}",
                outcome
                    .exit
                    .map(|status| status.python_exit_code())
                    .unwrap_or(0),
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            );
            return Err(CadProviderBuildError::new(message).into());
        };
        if !matches!(field(&payload, "ok"), Some(CanonicalValue::Bool(true))) {
            let message = match field(&payload, "error") {
                Some(value) => python_str(value),
                None => "FreeCAD refused the build".to_owned(),
            };
            return Err(CadProviderBuildError::new(op_refusal_message(&message, script)).into());
        }
        self.build_from_payload(&payload, &workspace)
            .map_err(CadProviderError::from)
    }

    fn build_from_payload(
        &self,
        payload: &CanonicalObject,
        workspace: &Path,
    ) -> Result<ProviderBuild, CadProviderBuildError> {
        let entries = match field(payload, "artifacts") {
            Some(CanonicalValue::Array(entries)) => entries,
            _ => {
                return Err(CadProviderBuildError::new("FreeCAD reported no artifacts"));
            }
        };
        let step_entry = artifact_entry(entries, "step");
        let native_entry = artifact_entry(entries, "fcstd");
        let Some(step_entry) = step_entry else {
            return Err(CadProviderBuildError::new(
                "FreeCAD did not export a STEP file",
            ));
        };
        let Some(native_entry) = native_entry else {
            return Err(CadProviderBuildError::new(
                "FreeCAD did not export an FCStd file",
            ));
        };
        let step_bytes = read_artifact(step_entry, workspace, self.max_step_bytes, "STEP")?;
        let native_bytes = read_artifact(native_entry, workspace, NATIVE_MAX_BYTES, "FCStd")?;
        let measured_dict = match field(payload, "measurement") {
            Some(CanonicalValue::Object(measured)) => Some(measured),
            _ => None,
        };
        let measured = MeasuredProperties::new(
            measured_dict.and_then(|measured| finite_or_none(field(measured, "volume_mm3"))),
            measured_dict.and_then(|measured| finite_or_none(field(measured, "area_mm2"))),
            field(payload, "bounds").and_then(bounds_or_none),
            None,
            None,
        )
        .map_err(|error: CadValueError| CadProviderBuildError::new(error.message()))?;
        let warnings = field(payload, "warnings")
            .map(string_tuple)
            .unwrap_or_default();
        let valid = match field(payload, "valid") {
            Some(CanonicalValue::Bool(value)) => Some(*value),
            _ => None,
        };
        let solid_count = match field(payload, "solid_count") {
            Some(CanonicalValue::Integer(value)) => {
                if *value < num_bigint::BigInt::from(0) {
                    None
                } else {
                    value.to_i64()
                }
            }
            _ => None,
        };
        let version = match field(payload, "freecad_version") {
            Some(CanonicalValue::Text(value)) => value.to_utf8().ok(),
            _ => None,
        };
        ProviderBuild::new(
            step_bytes,
            measured,
            version,
            warnings,
            Some(native_bytes),
            Some("fcstd".to_owned()),
            valid,
            solid_count,
        )
        .map_err(|error: CadValueError| CadProviderBuildError::new(error.message()))
    }

    /// Return the provider environment without credential-shaped keys.
    fn scrubbed_environment(&self) -> BTreeMap<String, String> {
        self.environ
            .iter()
            .filter(|(key, _)| {
                let upper = key.to_uppercase();
                !SENSITIVE_ENV_MARKERS
                    .iter()
                    .any(|marker| upper.contains(marker))
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    fn probe_environment(&self) -> Result<ChildEnvironment, kala_process::EnvironmentError> {
        let mut environment = self.scrubbed_environment();
        environment.remove("PYTHONPATH");
        child_environment(environment)
    }

    fn build_environment(
        &self,
        request_path: &Path,
        workspace: &Path,
        response_path: &Path,
    ) -> Result<ChildEnvironment, kala_process::EnvironmentError> {
        let mut environment = self.scrubbed_environment();
        environment.remove("PYTHONPATH");
        environment.insert(
            SYS_PATH_ENV_VAR.to_owned(),
            join_paths(self.driver_sys_path()),
        );
        environment.insert(
            REQUEST_ENV_VAR.to_owned(),
            request_path.to_string_lossy().into_owned(),
        );
        environment.insert(
            WORKSPACE_ENV_VAR.to_owned(),
            workspace.to_string_lossy().into_owned(),
        );
        environment.insert(
            OUTPUT_ENV_VAR.to_owned(),
            response_path.to_string_lossy().into_owned(),
        );
        environment.insert(MAX_STEP_ENV_VAR.to_owned(), self.max_step_bytes.to_string());
        environment.insert(
            MIN_ARTIFACT_BYTES_ENV_VAR.to_owned(),
            NATIVE_MAX_BYTES.to_string(),
        );
        environment.insert("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned());
        child_environment(environment)
    }

    /// The adapter search path handed to the driver, existing directories only.
    ///
    /// The adapter source comes first, then the single `site-packages-cpNNN`
    /// directory selected by [`freecad::bundled_abi_site_packages`]. A
    /// development checkout without vendored ABI directories yields only the
    /// source directories, exactly as before.
    fn driver_sys_path(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = match &self.sys_path {
            Some(paths) => paths.iter().filter(|path| path.is_dir()).cloned().collect(),
            None => discover_driver_sys_path(),
        };
        if let Some(site_packages) = &self.abi_site_packages {
            if site_packages.is_dir() && !paths.contains(site_packages) {
                paths.push(site_packages.clone());
            }
        }
        paths
    }

    fn resolve_executable(&self) -> Option<PathBuf> {
        if let Some(executable) = &self.executable {
            return executable.is_file().then(|| absolutize(executable));
        }
        let path = self.environ.get("PATH")?;
        for name in FREECAD_EXECUTABLE_NAMES {
            for entry in std::env::split_paths(path) {
                let candidate = entry.join(name);
                if is_executable_file(&candidate) {
                    return Some(absolutize(&candidate));
                }
            }
        }
        None
    }
}

impl CadProvider for FreeCadProvider {
    fn name(&self) -> &'static str {
        "freecad"
    }

    fn display_name(&self) -> &'static str {
        "FreeCAD"
    }

    fn measured_by(&self) -> &'static str {
        "FreeCAD"
    }

    fn mode(&self) -> &'static str {
        "external"
    }

    fn probe(&mut self) -> Availability {
        FreeCadProvider::probe(self)
    }

    fn build(&mut self, script: &ConstructionScript) -> Result<ProviderBuild, CadProviderError> {
        FreeCadProvider::build(self, script)
    }

    fn build_cancellable(
        &mut self,
        script: &ConstructionScript,
        cancel: &CancelToken,
    ) -> Result<ProviderBuild, CadProviderError> {
        FreeCadProvider::build_cancellable(self, script, cancel)
    }
}

/// Resolve the STEP import ceiling from an environment snapshot.
///
/// An absent or blank `KALA_STEP_IMPORT_MAX_BYTES` keeps the 64 MiB default; a
/// malformed or out-of-range value refuses rather than silently changing a
/// security boundary.
pub fn resolve_step_import_max_bytes(
    environ: &BTreeMap<String, String>,
) -> Result<usize, CadProviderConfigError> {
    let raw = environ.get(STEP_IMPORT_MAX_BYTES_ENV_VAR);
    match raw {
        None => validate_step_import_max_bytes(DEFAULT_MAX_STEP_BYTES),
        Some(value) if value.trim().is_empty() => {
            validate_step_import_max_bytes(DEFAULT_MAX_STEP_BYTES)
        }
        Some(value) => match parse_python_int(value.trim()) {
            Some(parsed)
                if (MIN_STEP_IMPORT_BYTES as i128..=MAX_STEP_IMPORT_BYTES as i128)
                    .contains(&parsed) =>
            {
                Ok(parsed as usize)
            }
            None => Err(CadProviderConfigError::new(format!(
                "{STEP_IMPORT_MAX_BYTES_ENV_VAR} must be an integer number of bytes"
            ))),
            Some(_) => Err(CadProviderConfigError::new(format!(
                "the STEP import limit must be between {MIN_STEP_IMPORT_BYTES} and \
                 {MAX_STEP_IMPORT_BYTES} bytes"
            ))),
        },
    }
}

/// Validate one STEP import ceiling.
pub fn validate_step_import_max_bytes(value: usize) -> Result<usize, CadProviderConfigError> {
    if !(MIN_STEP_IMPORT_BYTES..=MAX_STEP_IMPORT_BYTES).contains(&value) {
        return Err(CadProviderConfigError::new(format!(
            "the STEP import limit must be between {MIN_STEP_IMPORT_BYTES} and \
             {MAX_STEP_IMPORT_BYTES} bytes"
        )));
    }
    Ok(value)
}

/// Name the failing assembly op when the adapter message does not.
fn op_refusal_message(message: &str, script: &ConstructionScript) -> String {
    let lowered = message.to_lowercase();
    if lowered.contains("one valid solid") || lowered.contains("did not produce a solid") {
        if let Some(step) = script
            .assembly()
            .iter()
            .find(|step| matches!(step.op(), "fuse" | "compound"))
        {
            return format!("{message} (op: {})", step.op());
        }
    }
    message.to_owned()
}

fn artifact_entry<'a>(
    entries: &'a [CanonicalValue],
    export_format: &str,
) -> Option<&'a CanonicalObject> {
    entries.iter().find_map(|entry| match entry {
        CanonicalValue::Object(object) => match field(object, "format") {
            Some(CanonicalValue::Text(value))
                if value.to_utf8().ok().as_deref() == Some(export_format) =>
            {
                Some(object)
            }
            _ => None,
        },
        _ => None,
    })
}

fn read_artifact(
    entry: &CanonicalObject,
    workspace: &Path,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, CadProviderBuildError> {
    let raw_path = match field(entry, "path") {
        Some(CanonicalValue::Text(value)) => value,
        _ => {
            return Err(CadProviderBuildError::new(format!(
                "the FreeCAD {label} artifact descriptor is incomplete"
            )));
        }
    };
    let raw_digest = match field(entry, "digest") {
        Some(CanonicalValue::Text(value)) => value,
        _ => {
            return Err(CadProviderBuildError::new(format!(
                "the FreeCAD {label} artifact descriptor is incomplete"
            )));
        }
    };
    let Some(digest) = raw_digest.to_utf8().ok() else {
        return Err(CadProviderBuildError::new(format!(
            "the FreeCAD {label} artifact descriptor is incomplete"
        )));
    };
    let (raw_size, size_display) = match field(entry, "size_bytes") {
        Some(CanonicalValue::Integer(value)) if *value >= num_bigint::BigInt::from(0) => {
            (value.to_u64(), value.to_string())
        }
        _ => {
            return Err(CadProviderBuildError::new(format!(
                "the FreeCAD {label} artifact size is invalid"
            )));
        }
    };
    if raw_size.is_none_or(|size| size > limit as u64) {
        return Err(CadProviderBuildError::new(format!(
            "the FreeCAD {label} export is {size_display} bytes, above the configured \
             limit of {limit} bytes"
        )));
    }
    let raw_path = match raw_path.to_utf8() {
        Ok(value) => value,
        Err(_) => {
            return Err(CadProviderBuildError::new(format!(
                "the FreeCAD {label} artifact is outside the build workspace"
            )));
        }
    };
    let Some(path) = python_resolve(&raw_path) else {
        return Err(CadProviderBuildError::new(format!(
            "the FreeCAD {label} artifact is outside the build workspace"
        )));
    };
    let workspace_root = match std::fs::canonicalize(workspace) {
        Ok(root) => root,
        Err(_) => {
            return Err(CadProviderBuildError::new(format!(
                "the FreeCAD {label} artifact is outside the build workspace"
            )));
        }
    };
    if !path.starts_with(&workspace_root) {
        return Err(CadProviderBuildError::new(format!(
            "the FreeCAD {label} artifact is outside the build workspace"
        )));
    }
    let data = std::fs::read(&path).map_err(|error| {
        CadProviderBuildError::new(format!(
            "the FreeCAD {label} artifact could not be read: {}",
            io_error_name(&error)
        ))
    })?;
    if data.len() as u64 != raw_size.expect("checked above") {
        return Err(CadProviderBuildError::new(format!(
            "the FreeCAD {label} artifact changed size before it was read"
        )));
    }
    if sha256_hex(&data) != digest {
        return Err(CadProviderBuildError::new(format!(
            "the FreeCAD {label} artifact does not match the digest FreeCAD reported"
        )));
    }
    Ok(data)
}

/// Return a bounded, whitespace-collapsed tail of the FreeCAD stderr capture.
fn stderr_tail(raw: &[u8], limit: usize) -> String {
    let window = limit.saturating_mul(4);
    let start = raw.len().saturating_sub(window);
    let text = String::from_utf8_lossy(&raw[start..]);
    let joined = split_and_join(&text);
    let characters: Vec<char> = joined.chars().collect();
    if characters.len() <= limit {
        return joined;
    }
    characters[characters.len() - limit..].iter().collect()
}

fn read_response(path: &Path) -> Option<CanonicalObject> {
    let size = std::fs::metadata(path).ok()?.len();
    if size == 0 || size > MAX_RESPONSE_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let value = strict_json_loads(text.as_bytes()).ok()?;
    match &value {
        CanonicalValue::Object(object) => Some(object.clone()),
        _ => None,
    }
}

fn finite_or_none(value: Option<&CanonicalValue>) -> Option<f64> {
    match value? {
        CanonicalValue::Bool(_) => None,
        CanonicalValue::Integer(number) => number.to_f64().filter(|value| value.is_finite()),
        CanonicalValue::Float(number) => number.is_finite().then_some(*number),
        _ => None,
    }
}

fn bounds_or_none(value: &CanonicalValue) -> Option<MeasuredBounds> {
    let CanonicalValue::Object(object) = value else {
        return None;
    };
    let minimum = match field(object, "min") {
        Some(CanonicalValue::Array(values)) if values.len() == 3 => values,
        _ => return None,
    };
    let maximum = match field(object, "max") {
        Some(CanonicalValue::Array(values)) if values.len() == 3 => values,
        _ => return None,
    };
    let mut coordinates = [0.0; 6];
    for (index, value) in minimum.iter().chain(maximum.iter()).enumerate() {
        coordinates[index] = python_float(value)?;
    }
    if !coordinates.iter().all(|value| value.is_finite()) {
        return None;
    }
    MeasuredBounds::new(
        [coordinates[0], coordinates[1], coordinates[2]],
        [coordinates[3], coordinates[4], coordinates[5]],
    )
    .ok()
}

fn string_tuple(value: &CanonicalValue) -> Vec<String> {
    let CanonicalValue::Array(items) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            CanonicalValue::Text(text) => text.to_utf8().ok(),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .map(|text| truncate_chars(&text, 256))
        .collect()
}

/// The exact canonical request JSON the FreeCAD driver receives.
///
/// This is Python's `json.dumps({"script": script.model_dump(mode="json",
/// exclude_none=True)}, sort_keys=True, separators=(",", ":"),
/// ensure_ascii=True)`.
pub fn canonical_request_json(
    script: &ConstructionScript,
) -> Result<String, CadProviderBuildError> {
    let wire = drop_none(script.to_wire_value());
    let script_value = wire.to_canonical_value().map_err(|error| {
        CadProviderBuildError::new(format!(
            "the construction script could not be serialized: {}",
            error.message()
        ))
    })?;
    let mut request = CanonicalObject::new();
    request.insert(JsonText::new("script"), script_value);
    canonical_dumps(&CanonicalValue::Object(request), &Limits::default()).map_err(|error| {
        CadProviderBuildError::new(format!(
            "the construction script could not be serialized: {error}"
        ))
    })
}

/// Apply Pydantic's `exclude_none=True` to a model dump.
///
/// Array elements are left alone: `exclude_none` removes `None` fields, not
/// list members.
fn drop_none(value: WireValue) -> WireValue {
    match value {
        WireValue::Object(fields) => WireValue::Object(
            fields
                .into_iter()
                .filter(|(_, value)| !matches!(value, WireValue::Null))
                .map(|(name, value)| (name, drop_none(value)))
                .collect(),
        ),
        WireValue::Array(values) => WireValue::Array(
            values
                .into_iter()
                .map(|value| match value {
                    WireValue::Object(_) => drop_none(value),
                    other => other,
                })
                .collect(),
        ),
        other => other,
    }
}

fn field<'a>(object: &'a CanonicalObject, name: &str) -> Option<&'a CanonicalValue> {
    object.get(&JsonText::new(name))
}

fn canonical_text(object: &CanonicalObject, name: &str) -> Option<String> {
    match field(object, name)? {
        CanonicalValue::Text(value) => value.to_utf8().ok(),
        _ => None,
    }
}

fn child_environment(
    entries: BTreeMap<String, String>,
) -> Result<ChildEnvironment, kala_process::EnvironmentError> {
    let validated = entries
        .into_iter()
        .filter(|(key, _)| valid_environment_key(key))
        .filter_map(|(key, value)| EnvironmentEntry::new(key, value).ok());
    ChildEnvironment::from_entries(validated)
}

fn valid_environment_key(key: &str) -> bool {
    let mut characters = key.chars();
    match characters.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    key.len() <= 64
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn run_bounded(
    executable: &Path,
    arguments: &[OsString],
    working_directory: &Path,
    environment: ChildEnvironment,
    wall_seconds: f64,
) -> Result<ProcessOutcome, ProcessError> {
    run_bounded_with_cancel(
        executable,
        arguments,
        working_directory,
        environment,
        wall_seconds,
        &CancelToken::new(),
    )
}

fn run_bounded_with_cancel(
    executable: &Path,
    arguments: &[OsString],
    working_directory: &Path,
    environment: ChildEnvironment,
    wall_seconds: f64,
    cancel: &CancelToken,
) -> Result<ProcessOutcome, ProcessError> {
    let limits = ProcessLimits {
        wall_timeout: Duration::from_secs_f64(wall_seconds),
        bootstrap_timeout: Duration::from_secs_f64(wall_seconds.min(BOOTSTRAP_TIMEOUT_SECONDS)),
        termination_timeout: Duration::from_secs_f64(TERMINATION_TIMEOUT_SECONDS),
        max_stdout_bytes: kala_process::MAX_STDOUT_BYTES,
        max_stderr_bytes: kala_process::MAX_STDERR_BYTES,
        // Python applies no POSIX resource limits to this subprocess, and a
        // provider export may legitimately exceed the tools executor's 8 MiB
        // file-size default.
        enforce_resource_limits: false,
        ..ProcessLimits::default()
    };
    let spec = ProcessSpec::new(executable, working_directory)?
        .args(arguments.iter().cloned())
        .with_environment(environment)
        .with_limits(limits);
    supervise(&spec, b"", cancel)
}

fn current_directory() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn version_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"FreeCAD\s+([0-9]+(?:\.[0-9]+){1,3}(?:[-+][0-9A-Za-z.]+)?)")
            .expect("the FreeCAD version pattern compiles")
    })
}

/// Derive the pip/CPython ABI tag from `sys.implementation.cache_tag`.
///
/// `cpython-314` becomes `cp314`; a free-threaded `cpython-313t` becomes
/// `cp313t`. An unrecognised tag yields `None` and is reported honestly.
fn abi_from_cache_tag(tag: &str) -> Option<String> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r"^cpython-([0-9]+)(t?)$").expect("the CPython cache-tag pattern compiles")
    });
    let captures = pattern.captures(tag)?;
    Some(format!(
        "cp{}{}",
        captures.get(1)?.as_str(),
        captures.get(2).map_or("", |value| value.as_str())
    ))
}

fn probe_error_name(error: &ProcessError, executable: &Path) -> &'static str {
    match error {
        ProcessError::Spawn { .. } => {
            if !executable.exists() {
                "FileNotFoundError"
            } else if !is_executable_file(executable) {
                "PermissionError"
            } else {
                "OSError"
            }
        }
        _ => "OSError",
    }
}

fn environment_error_name(error: &kala_process::EnvironmentError) -> &'static str {
    use kala_process::EnvironmentError;
    match error {
        EnvironmentError::InvalidKey
        | EnvironmentError::NullValue
        | EnvironmentError::DuplicateKey
        | EnvironmentError::DisallowedKey(_) => "ValueError",
        EnvironmentError::ProtectedKey(_) | EnvironmentError::Workspace(_) => "OSError",
    }
}

fn is_executable_file(path: &Path) -> bool {
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

fn absolutize(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn discover_driver_sys_path() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(directory) = std::env::current_dir() {
        roots.extend(directory.ancestors().map(Path::to_path_buf));
    }
    if let Ok(executable) = std::env::current_exe() {
        roots.extend(executable.ancestors().map(Path::to_path_buf));
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    for root in roots {
        for candidate in [
            root.join("adapters").join("freecad").join("src"),
            root.join("src"),
        ] {
            push_unique(&mut paths, candidate);
        }
        let venv_lib = root.join(".venv").join("lib");
        let Ok(entries) = std::fs::read_dir(&venv_lib) else {
            continue;
        };
        let mut site_packages: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("python"))
            .map(|entry| entry.path().join("site-packages"))
            .filter(|path| path.is_dir())
            .collect();
        site_packages.sort();
        for path in site_packages {
            push_unique(&mut paths, path);
        }
    }
    paths
}

fn push_unique(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if candidate.is_dir() && !paths.contains(&candidate) {
        paths.push(candidate);
    }
}

fn join_paths(paths: Vec<PathBuf>) -> String {
    let separator = if cfg!(windows) { ';' } else { ':' };
    paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<String>>()
        .join(&separator.to_string())
}

/// Resolve a path like Python's `Path.resolve()` (non-strict).
///
/// Existing components are canonicalized (following symlinks); missing
/// components are appended lexically after `.`/`..` normalization.
fn python_resolve(raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => resolved.push(name),
        }
        if resolved.exists() {
            if let Ok(canonical) = std::fs::canonicalize(&resolved) {
                resolved = canonical;
            }
        }
    }
    Some(resolved)
}

fn io_error_name(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "FileNotFoundError",
        io::ErrorKind::PermissionDenied => "PermissionError",
        io::ErrorKind::AlreadyExists => "FileExistsError",
        io::ErrorKind::IsADirectory => "IsADirectoryError",
        io::ErrorKind::NotADirectory => "NotADirectoryError",
        io::ErrorKind::InvalidInput => "ValueError",
        io::ErrorKind::OutOfMemory => "MemoryError",
        _ => "OSError",
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn split_and_join(text: &str) -> String {
    text.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// Python `format(value, "g")` with the default six significant digits.
fn format_g(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if !value.is_finite() {
        return value.to_string();
    }
    let precision: i32 = 6;
    let mut exponent = value.abs().log10().floor() as i32;
    let normalised = value.abs() / 10_f64.powi(exponent);
    if normalised >= 10.0 {
        exponent += 1;
    } else if normalised < 1.0 {
        exponent -= 1;
    }
    if exponent < -4 || exponent >= precision {
        let text = format!("{:.*e}", (precision - 1) as usize, value);
        return python_scientific(&text);
    }
    let decimals = (precision - 1 - exponent).max(0) as usize;
    let mut text = format!("{:.*}", decimals, value);
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

fn python_scientific(text: &str) -> String {
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text.to_owned();
    };
    let mut mantissa = mantissa.to_owned();
    if mantissa.contains('.') {
        while mantissa.ends_with('0') {
            mantissa.pop();
        }
        if mantissa.ends_with('.') {
            mantissa.pop();
        }
    }
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => ("+", exponent),
    };
    format!("{mantissa}e{sign}{digits:0>2}")
}

/// CPython `repr()` for the path strings used in probe messages.
fn python_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::new();
    rendered.push(quote);
    for character in value.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            character if character == quote => {
                rendered.push('\\');
                rendered.push(character);
            }
            character if (character as u32) < 0x20 || (character as u32) == 0x7f => {
                let _ = write!(rendered, "\\x{:02x}", character as u32);
            }
            character => rendered.push(character),
        }
    }
    rendered.push(quote);
    rendered
}

/// Python `str()` for the JSON values a driver can put in an error field.
fn python_str(value: &CanonicalValue) -> String {
    match value {
        CanonicalValue::Text(text) => text.to_string_lossy(),
        CanonicalValue::Bool(true) => "True".to_owned(),
        CanonicalValue::Bool(false) => "False".to_owned(),
        CanonicalValue::Null => "None".to_owned(),
        CanonicalValue::Integer(number) => number.to_string(),
        CanonicalValue::Float(number) => {
            kala_json::format_float(*number).unwrap_or_else(|_| number.to_string())
        }
        other => canonical_dumps(other, &Limits::default()).unwrap_or_default(),
    }
}

/// Python `float()` for the values a driver can put in bounds.
fn python_float(value: &CanonicalValue) -> Option<f64> {
    match value {
        CanonicalValue::Bool(true) => Some(1.0),
        CanonicalValue::Bool(false) => Some(0.0),
        CanonicalValue::Integer(number) => number.to_f64(),
        CanonicalValue::Float(number) => Some(*number),
        CanonicalValue::Text(text) => parse_python_float(&text.to_utf8().ok()?),
        _ => None,
    }
}

fn parse_python_float(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lowered = trimmed.to_ascii_lowercase();
    let unsigned = lowered.strip_prefix(['+', '-']).unwrap_or(&lowered);
    if matches!(unsigned, "inf" | "infinity") {
        return Some(if lowered.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    if matches!(unsigned, "nan") {
        return Some(f64::NAN);
    }
    // Python accepts single underscores between digits.
    if trimmed.contains('_') {
        let mut cleaned = String::with_capacity(trimmed.len());
        let mut previous_digit = false;
        for character in trimmed.chars() {
            if character == '_' {
                if !previous_digit {
                    return None;
                }
                previous_digit = false;
                continue;
            }
            previous_digit = character.is_ascii_digit();
            cleaned.push(character);
        }
        if trimmed.ends_with('_') {
            return None;
        }
        return cleaned.parse::<f64>().ok();
    }
    trimmed.parse::<f64>().ok()
}

/// Parse a Python `int()` string, or `None` when it is not an integer.
///
/// Values beyond `i128` are reported as `Some(i128::MAX)` so the caller's
/// range check produces the range message instead of the syntax message,
/// which is what Python does with an arbitrary-precision integer.
fn parse_python_int(text: &str) -> Option<i128> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (negative, body) = match trimmed.as_bytes()[0] {
        b'-' => (true, &trimmed[1..]),
        b'+' => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    if body.is_empty() {
        return None;
    }
    let mut value: i128 = 0;
    let mut previous_digit = false;
    let mut overflow = false;
    for character in body.chars() {
        if character == '_' {
            if !previous_digit {
                return None;
            }
            previous_digit = false;
            continue;
        }
        let digit = character.to_digit(10)?;
        previous_digit = true;
        if overflow {
            continue;
        }
        value = value
            .checked_mul(10)
            .and_then(|current| current.checked_add(i128::from(digit)))
            .unwrap_or_else(|| {
                overflow = true;
                i128::MAX
            });
    }
    if !previous_digit {
        return None;
    }
    if overflow {
        return Some(i128::MAX);
    }
    Some(if negative { -value } else { value })
}
