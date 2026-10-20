//! The strict-profile probe: the child that builds the jail and the parent
//! that decides whether the profile may be advertised.
//!
//! The parent launches the probe helper through the accepted `kala-process`
//! supervisor, so the launch, the wall-clock deadline, the cancellation token
//! and the output bounds are the ones the rest of the backend already uses.
//! The helper itself is native: no interpreter is started at product time.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kala_process::{
    CancelToken, ChildEnvironment, ProcessEnd, ProcessLimits, ProcessSpec, supervise,
};

use crate::constants::{
    CANARY_PAYLOAD, MIN_DEVICE_BYTES, PROBE_FAILURE_EXIT, PROBE_TIMEOUT_SECONDS,
    STRICT_OPTIONAL_CONTROLS, STRICT_REQUIRED_CONTROLS,
};
use crate::error::{FailureClass, SandboxProbeError};
use crate::isolation::isolate_process;
use crate::json::{JsonValue, dump_float};
use crate::parse::parse_json;
use crate::path::{posix_text, resolve_strict_false};
use crate::probe::{
    SandboxProbe, SandboxVerdict, judge_sandbox, truncate_detail, verify_controls_with,
};
use crate::profile::IsolationProfile;
use crate::spec::{SandboxSpec, build_sandbox_spec};

/// The options of [`probe_sandbox`].
#[derive(Debug, Clone)]
pub struct ProbeOptions {
    /// The native probe helper that builds the jail.
    pub helper: PathBuf,
    /// The host roots the worker needs read-only.
    pub import_roots: Vec<PathBuf>,
    /// The scratch tmpfs size to request.
    pub scratch_bytes: i64,
    /// The wall-clock deadline of the probe.
    pub timeout: Duration,
}

impl ProbeOptions {
    /// The compatibility defaults: the documented scratch size and the
    /// documented probe timeout.
    pub fn new(helper: impl Into<PathBuf>) -> Self {
        Self {
            helper: helper.into(),
            import_roots: Vec::new(),
            scratch_bytes: MIN_DEVICE_BYTES,
            timeout: Duration::from_secs_f64(PROBE_TIMEOUT_SECONDS),
        }
    }
}

/// Build the jail in this process and report what really worked.
///
/// The exit code is the Python one: `0` when every required control was
/// verified, [`PROBE_FAILURE_EXIT`] otherwise. The probe result is written to
/// standard output as one JSON line; a failed probe that could not verify a
/// required control writes nothing at all, exactly like the Python helper.
pub fn run_probe(spec_payload: &str, canary_payload: &str) -> i32 {
    let (spec, canary) = match probe_entry(spec_payload, canary_payload) {
        Ok(entry) => entry,
        Err(error) => {
            emit_probe_result(&[], error.detail());
            return PROBE_FAILURE_EXIT;
        }
    };
    let established = match isolate_process(&spec) {
        Ok(established) => established,
        Err(error) => {
            emit_probe_result(&[], error.detail());
            return PROBE_FAILURE_EXIT;
        }
    };
    let (verified, _failed) = verify_controls_with(
        &spec,
        &canary,
        &posix_text(&spec.workspace),
        Some(&established),
    );
    let unavailable: Vec<&str> = spec
        .controls
        .iter()
        .filter(|control| !verified.iter().any(|value| value == *control))
        .map(String::as_str)
        .collect();
    let detail = if unavailable.is_empty() {
        String::new()
    } else {
        format!("not enforced: {}", unavailable.join(", "))
    };
    if !STRICT_REQUIRED_CONTROLS
        .iter()
        .all(|control| verified.iter().any(|value| value == control))
    {
        return PROBE_FAILURE_EXIT;
    }
    emit_probe_result(&verified, &detail);
    0
}

/// Decode the probe payload.
pub fn probe_entry(
    spec_payload: &str,
    canary_payload: &str,
) -> Result<(SandboxSpec, String), SandboxProbeError> {
    let decoded = parse_json(spec_payload).map_err(|_| SandboxProbeError::malformed())?;
    let spec = SandboxSpec::from_payload(&decoded).map_err(|_| SandboxProbeError::malformed())?;
    let canary_value = parse_json(canary_payload).map_err(|_| SandboxProbeError::malformed())?;
    let canary = python_text(&canary_value).ok_or_else(SandboxProbeError::malformed)?;
    Ok((spec, canary))
}

/// Write the probe result as one JSON line.
pub fn emit_probe_result(controls: &[String], detail: &str) {
    let mut names: Vec<String> = controls.to_vec();
    names.sort();
    let payload = JsonValue::object(vec![
        (
            "controls".to_owned(),
            JsonValue::Array(
                names
                    .iter()
                    .map(|control| JsonValue::Text(control.clone()))
                    .collect(),
            ),
        ),
        (
            "detail".to_owned(),
            JsonValue::Text(truncate_detail(detail)),
        ),
    ]);
    println!("{}", payload.dump());
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

/// Probe whether the strict Linux profile can be established on this host.
///
/// The probe runs in a throwaway child that builds exactly the jail the
/// executor would build and then verifies, from the inside, that the network
/// is unreachable, the host filesystem is invisible and the workspace is
/// writable. Only controls that the probe observed are returned.
pub fn probe_sandbox(options: &ProbeOptions) -> SandboxProbe {
    if !cfg!(target_os = "linux") {
        return SandboxProbe::unavailable("the strict profile requires Linux namespaces", 0.0);
    }
    let helper = match std::fs::canonicalize(&options.helper) {
        Ok(helper) if helper.is_file() => helper,
        _ => {
            return SandboxProbe::unavailable(
                "no usable interpreter is available for the sandbox probe",
                0.0,
            );
        }
    };
    let started = Instant::now();
    let scratch = match tempfile::Builder::new()
        .prefix("kala-sandbox-probe-")
        .tempdir()
    {
        Ok(directory) => directory,
        Err(_) => return could_not_start(FailureClass::Os, elapsed_ms(&started)),
    };
    let root = scratch.path().join("root");
    let workspace = scratch.path().join("workspace");
    if create_private_directory(&workspace).is_err() {
        return could_not_start(FailureClass::Os, elapsed_ms(&started));
    }
    let canary = scratch.path().join("canary");
    if std::fs::write(&canary, CANARY_PAYLOAD).is_err() {
        return could_not_start(FailureClass::Os, elapsed_ms(&started));
    }
    let controls: BTreeSet<String> = STRICT_REQUIRED_CONTROLS
        .iter()
        .chain(STRICT_OPTIONAL_CONTROLS.iter())
        .map(|control| (*control).to_owned())
        .collect();
    let spec = match build_sandbox_spec(
        &controls,
        &root,
        &workspace,
        &options.import_roots,
        options.scratch_bytes,
    ) {
        Ok(spec) => spec,
        Err(_error) => {
            return could_not_start(FailureClass::Setup, elapsed_ms(&started));
        }
    };
    if create_private_directory(&root).is_err() {
        return could_not_start(FailureClass::Os, elapsed_ms(&started));
    }
    let environment = match ChildEnvironment::minimal(&workspace) {
        Ok(environment) => environment,
        Err(_) => return could_not_start(FailureClass::Os, elapsed_ms(&started)),
    };
    let limits = ProcessLimits {
        wall_timeout: options.timeout,
        bootstrap_timeout: options.timeout.min(Duration::from_secs(5)),
        termination_timeout: Duration::from_secs(5),
        ..ProcessLimits::default()
    };
    let mut process = match ProcessSpec::new(&helper, &workspace) {
        Ok(process) => process,
        Err(error) => {
            return could_not_start(process_failure_class(&error), elapsed_ms(&started));
        }
    };
    let canary_path = posix_text(&resolve_strict_false(&canary));
    process = process
        .with_environment(environment)
        .with_limits(limits)
        .arg(spec.to_json())
        .arg(JsonValue::Text(canary_path).dump());
    let outcome = supervise(&process, &[], &CancelToken::new());
    let elapsed = elapsed_ms(&started);
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => return could_not_start(process_failure_class(&error), elapsed),
    };
    match outcome.end {
        ProcessEnd::Exited(status) if status.code == 0 => {}
        ProcessEnd::TimedOut(_) => {
            return SandboxProbe::unavailable("the sandbox probe did not finish", elapsed);
        }
        ProcessEnd::OutputLimitExceeded(kala_process::OutputStream::Stdout) => {
            return SandboxProbe::unavailable("the sandbox probe returned no result", elapsed);
        }
        _ => {
            return SandboxProbe::unavailable(
                "the strict profile is unavailable on this host",
                elapsed,
            );
        }
    }
    let stdout = outcome.stdout.to_string_lossy();
    let stdout = stdout.trim();
    let decoded = if stdout.is_empty() {
        serde_json::json!({})
    } else {
        match parse_json(stdout) {
            Ok(value) => value,
            Err(_) => {
                return SandboxProbe::unavailable("the sandbox probe returned no result", elapsed);
            }
        }
    };
    let controls: Vec<String> = decoded
        .get("controls")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str())
                .filter(|name| {
                    STRICT_REQUIRED_CONTROLS.contains(name)
                        || STRICT_OPTIONAL_CONTROLS.contains(name)
                })
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if !STRICT_REQUIRED_CONTROLS
        .iter()
        .all(|control| controls.iter().any(|value| value == control))
    {
        return SandboxProbe::unavailable("a required sandbox control was not verified", elapsed);
    }
    let mut controls = controls;
    controls.sort();
    controls.dedup();
    SandboxProbe {
        available: true,
        platform: crate::platform_name(),
        controls,
        detail: String::new(),
        elapsed_ms: elapsed,
    }
}

/// Decide whether the configured profile can back a side-effecting tool.
///
/// This is a convenience wrapper over [`judge_sandbox`] for callers that hold
/// a profile and an optional probe.
pub fn judge(profile: IsolationProfile, probe: Option<&SandboxProbe>) -> SandboxVerdict {
    judge_sandbox(profile, probe)
}

/// The Python exception class a process failure maps onto.
fn process_failure_class(error: &kala_process::ProcessError) -> FailureClass {
    use kala_process::ProcessError;
    match error {
        ProcessError::InvalidSpec(_) => FailureClass::Value,
        ProcessError::SandboxSetup(_) => FailureClass::Setup,
        ProcessError::Workspace(_)
        | ProcessError::Environment(_)
        | ProcessError::Spawn { .. }
        | ProcessError::Containment(_)
        | ProcessError::Bootstrap(_)
        | ProcessError::Cleanup(_)
        | ProcessError::Wait(_)
        | ProcessError::Io(_) => FailureClass::Os,
        ProcessError::Internal(_) => FailureClass::Runtime,
    }
}

/// Create a directory that only its owner may enter.
///
/// The Python probe creates the jail root and the workspace with the mode
/// `0o700`; the port does the same.
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The refusal of a probe that could not be started.
///
/// The wording is the Python one: the failure class is reported, never the
/// host detail.
fn could_not_start(class: FailureClass, elapsed_ms: f64) -> SandboxProbe {
    SandboxProbe::unavailable(
        &format!("the sandbox probe could not start: {}", class.as_str()),
        elapsed_ms,
    )
}

fn elapsed_ms(started: &Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

/// The Python `str()` of a decoded JSON value.
fn python_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Bool(true) => Some("True".to_owned()),
        serde_json::Value::Bool(false) => Some("False".to_owned()),
        serde_json::Value::Null => Some("None".to_owned()),
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Some(integer.to_string())
            } else {
                number.as_f64().map(dump_float)
            }
        }
        _ => None,
    }
}

/// The path of the probe helper next to the running executable.
///
/// The native init helper is a drop-in replacement for the probe helper: it
/// answers the same probe protocol, so it is used when the probe helper is
/// not installed.
pub fn discover_helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KALA_SANDBOX_PROBE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let current = std::env::current_exe().ok()?;
    let directory = current.parent()?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for stem in ["kala-sandbox-probe", "kala-sandbox-init"] {
        candidates.push(directory.join(exe_name(stem)));
        candidates.push(directory.parent().unwrap_or(directory).join(exe_name(stem)));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

/// The path of the jail root and the workspace of a probe, for tests.
pub fn probe_paths(scratch: &Path) -> (PathBuf, PathBuf) {
    (scratch.join("root"), scratch.join("workspace"))
}
