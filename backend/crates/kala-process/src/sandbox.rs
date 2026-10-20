//! Portable and strict Linux isolation descriptions, probing and judging.
//!
//! This module is the native port of `kala_agent.tools.sandbox`. The strict
//! profile requires a Linux namespace/mount/chroot jail; Windows always reports
//! it unavailable. Nothing here executes Python.

use std::collections::BTreeSet;
#[cfg(target_os = "linux")]
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::error::{SandboxProbeError, SandboxSetupError};
use crate::limits::ProcessLimits;

/// `unshare(CLONE_NEWNS)`.
pub const CLONE_NEWNS: i32 = 0x0002_0000;
/// `unshare(CLONE_NEWUTS)`.
pub const CLONE_NEWUTS: i32 = 0x0400_0000;
/// `unshare(CLONE_NEWIPC)`.
pub const CLONE_NEWIPC: i32 = 0x0800_0000;
/// `unshare(CLONE_NEWUSER)`.
pub const CLONE_NEWUSER: i32 = 0x1000_0000;
/// `unshare(CLONE_NEWPID)`.
pub const CLONE_NEWPID: i32 = 0x2000_0000;
/// `unshare(CLONE_NEWNET)`.
pub const CLONE_NEWNET: i32 = 0x4000_0000;

/// `MS_RDONLY`.
pub const MS_RDONLY: u64 = 1;
/// `MS_NOSUID`.
pub const MS_NOSUID: u64 = 2;
/// `MS_NODEV`.
pub const MS_NODEV: u64 = 4;
/// `MS_NOEXEC`.
pub const MS_NOEXEC: u64 = 8;
/// `MS_REMOUNT`.
pub const MS_REMOUNT: u64 = 32;
/// `MS_BIND`.
pub const MS_BIND: u64 = 4_096;
/// `MS_REC`.
pub const MS_REC: u64 = 16_384;
/// `MS_PRIVATE`.
pub const MS_PRIVATE: u64 = 1 << 18;

/// Exit code the Python sandbox uses to signal that isolation failed.
pub const ISOLATION_FAILURE_EXIT: i32 = 121;
/// Exit code the Python probe uses for a failed probe.
pub const PROBE_FAILURE_EXIT: i32 = 122;
/// Python probe wall-clock bound.
pub const PROBE_TIMEOUT_SECONDS: f64 = 30.0;
const MAX_PROBE_DETAIL: usize = 480;
const CANARY_PAYLOAD: &[u8] = b"kala-sandbox-canary";
const MIN_DEVICE_BYTES: i64 = 1 << 20;

/// Controls a strict Linux jail must establish.
pub const STRICT_REQUIRED_CONTROLS: &[&str] = &[
    "chroot_workspace_only",
    "mount_namespace",
    "network_isolation",
    "readonly_system_bindings",
    "user_namespace",
    "workspace_read_write",
];

/// Controls a strict Linux jail may additionally establish.
///
/// `pid_namespace` is listed because the Python source lists it, but the native
/// launcher does not implement the double-fork PID-1 transition yet. It is
/// never advertised and a specification that requests it fails closed.
pub const STRICT_OPTIONAL_CONTROLS: &[&str] = &[
    "device_nodes_minimal",
    "ipc_namespace",
    "pid_namespace",
    "procfs",
    "scratch_tmpfs",
    "uts_namespace",
];

const DEVICE_NODES: &[&str] = &[
    "/dev/full",
    "/dev/null",
    "/dev/random",
    "/dev/urandom",
    "/dev/zero",
];
const SCRATCH_PATHS: &[&str] = &["/dev/shm", "/tmp"];
const OPTIONAL_SYSTEM_DIRECTORIES: &[&str] = &["/bin", "/lib", "/lib32", "/lib64", "/sbin", "/usr"];
const OPTIONAL_SYSTEM_FILES: &[&str] =
    &["/etc/ld.so.cache", "/etc/localtime", "/etc/nsswitch.conf"];

/// The configured isolation profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationProfile {
    /// Process confinement only. It is not a filesystem jail.
    Portable,
    /// Linux namespace/mount/chroot isolation, available only when probed.
    StrictLinux,
}

impl IsolationProfile {
    /// The Python `StrEnum` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::StrictLinux => "strict_linux",
        }
    }
}

/// The outcome of a preflight probe of the strict profile.
///
/// `controls` contains only controls the probe established and verified inside
/// a throwaway native child.
#[derive(Debug, Clone, PartialEq)]
pub struct SandboxProbe {
    /// Whether every required control was verified.
    pub available: bool,
    /// The host platform description.
    pub platform: String,
    /// Verified controls, sorted.
    pub controls: Vec<String>,
    /// Sanitised detail for an unavailable probe.
    pub detail: String,
    /// Probe elapsed milliseconds.
    pub elapsed_ms: f64,
}

impl SandboxProbe {
    /// The exact Python payload shape.
    pub fn to_payload(&self) -> Value {
        json!({
            "available": self.available,
            "controls": self.controls,
            "detail": self.detail,
            "elapsed_ms": round_three(self.elapsed_ms),
            "platform": self.platform,
        })
    }

    /// Decode and validate a probe payload.
    pub fn from_payload(payload: &Value) -> Result<Self, SandboxProbeError> {
        let object = payload
            .as_object()
            .ok_or_else(|| SandboxProbeError::new("the sandbox probe payload is malformed"))?;
        let available = object
            .get("available")
            .and_then(Value::as_bool)
            .ok_or_else(|| SandboxProbeError::new("the sandbox probe payload is malformed"))?;
        let platform = object
            .get("platform")
            .and_then(Value::as_str)
            .ok_or_else(|| SandboxProbeError::new("the sandbox probe payload is malformed"))?
            .to_owned();
        let detail = object
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let elapsed_ms = object
            .get("elapsed_ms")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let controls = object
            .get("controls")
            .and_then(Value::as_array)
            .ok_or_else(|| SandboxProbeError::new("the sandbox probe payload is malformed"))?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        Ok(Self {
            available,
            platform,
            controls,
            detail,
            elapsed_ms,
        })
    }
}

/// A resolved, minimal description of the jail a child must build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxSpec {
    /// Verified controls this specification requests.
    pub controls: BTreeSet<String>,
    /// Host-side jail root.
    pub root: PathBuf,
    /// Host-side workspace that becomes writable inside the jail.
    pub workspace: PathBuf,
    /// Host paths bind-mounted read-only.
    pub readonly_bindings: Vec<String>,
    /// Host paths bind-mounted read-write.
    pub writable_bindings: Vec<String>,
    /// Host device nodes bind-mounted into the jail.
    pub device_bindings: Vec<String>,
    /// Paths that receive a private tmpfs.
    pub scratch_paths: Vec<String>,
    /// Size of each scratch tmpfs in bytes.
    pub scratch_bytes: i64,
}

impl SandboxSpec {
    /// The exact Python payload shape.
    pub fn to_payload(&self) -> Value {
        json!({
            "controls": self.controls.iter().cloned().collect::<Vec<_>>(),
            "device_bindings": self.device_bindings,
            "readonly_bindings": self.readonly_bindings,
            "root": path_text(&self.root),
            "scratch_bytes": self.scratch_bytes,
            "scratch_paths": self.scratch_paths,
            "writable_bindings": self.writable_bindings,
            "workspace": path_text(&self.workspace),
        })
    }

    /// Decode a specification, rejecting every type error with the Python
    /// malformed-payload error.
    pub fn from_payload(payload: &Value) -> Result<Self, SandboxSetupError> {
        let object = payload
            .as_object()
            .ok_or_else(SandboxSetupError::malformed)?;
        let controls = string_sequence(object.get("controls"))
            .ok_or_else(SandboxSetupError::malformed)?
            .into_iter()
            .collect();
        let root = path_value(object.get("root")).ok_or_else(SandboxSetupError::malformed)?;
        let workspace =
            path_value(object.get("workspace")).ok_or_else(SandboxSetupError::malformed)?;
        let readonly_bindings = string_sequence(object.get("readonly_bindings"))
            .ok_or_else(SandboxSetupError::malformed)?;
        let writable_bindings = string_sequence(object.get("writable_bindings"))
            .ok_or_else(SandboxSetupError::malformed)?;
        let device_bindings = string_sequence(object.get("device_bindings"))
            .ok_or_else(SandboxSetupError::malformed)?;
        let scratch_paths = string_sequence(object.get("scratch_paths"))
            .ok_or_else(SandboxSetupError::malformed)?;
        let scratch_bytes = object
            .get("scratch_bytes")
            .and_then(Value::as_i64)
            .ok_or_else(SandboxSetupError::malformed)?;
        Ok(Self {
            controls,
            root,
            workspace,
            readonly_bindings,
            writable_bindings,
            device_bindings,
            scratch_paths,
            scratch_bytes,
        })
    }

    /// The controls this specification requests, sorted.
    pub fn controls(&self) -> Vec<String> {
        self.controls.iter().cloned().collect()
    }

    /// Whether every required strict control is present.
    pub fn has_required_controls(&self) -> bool {
        STRICT_REQUIRED_CONTROLS
            .iter()
            .all(|control| self.controls.contains(*control))
    }

    /// Reject unknown controls, missing required controls and the unimplemented
    /// PID-namespace transition before anything is launched.
    pub fn validate_strict(&self) -> Result<(), SandboxSetupError> {
        for control in &self.controls {
            if !STRICT_REQUIRED_CONTROLS.contains(&control.as_str())
                && !STRICT_OPTIONAL_CONTROLS.contains(&control.as_str())
            {
                return Err(SandboxSetupError::new(format!(
                    "the sandbox specification requests an unknown control: {control}"
                )));
            }
        }
        let missing: Vec<&str> = STRICT_REQUIRED_CONTROLS
            .iter()
            .copied()
            .filter(|control| !self.controls.contains(*control))
            .collect();
        if !missing.is_empty() {
            return Err(SandboxSetupError::new(format!(
                "the strict profile requires controls that were not probed: {}",
                missing.join(", ")
            )));
        }
        if self.controls.contains("pid_namespace") {
            return Err(SandboxSetupError::new(
                "pid_namespace is not implemented by the native launcher; \
                 the PID-1 double-fork transition is pending",
            ));
        }
        if !safe_host_path(&self.root) || !safe_host_path(&self.workspace) {
            return Err(SandboxSetupError::new(
                "sandbox paths must be absolute and free of traversal segments",
            ));
        }
        if path_text(&self.root).trim_end_matches('/').is_empty()
            || path_text(&self.workspace).trim_end_matches('/').is_empty()
        {
            return Err(SandboxSetupError::new(
                "the sandbox root and workspace must not be the host root",
            ));
        }
        Ok(())
    }
}

/// The result of asking whether a side effect may proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxVerdict {
    /// Whether the configured profile can back the side effect.
    pub allowed: bool,
    /// A sanitised reason when refused.
    pub reason: String,
    /// The verified controls that back an allowed strict verdict.
    pub controls: Vec<String>,
}

impl SandboxVerdict {
    /// The exact Python payload shape.
    pub fn to_payload(&self) -> Value {
        json!({
            "allowed": self.allowed,
            "controls": self.controls,
            "reason": self.reason,
        })
    }
}

/// The Python-compatible platform description.
pub fn platform_name() -> String {
    #[cfg(target_os = "linux")]
    {
        format!("linux-{}", std::env::consts::ARCH)
    }
    #[cfg(target_os = "macos")]
    {
        "darwin".to_owned()
    }
    #[cfg(target_os = "windows")]
    {
        "win32".to_owned()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        std::env::consts::OS.to_owned()
    }
}

/// Resolve read-only host paths a native worker may need.
///
/// Unlike the Python helper, this deliberately does **not** add interpreter,
/// stdlib or site-package paths. The native-worker bootstrap replaces that
/// interpreter bootstrap.
pub fn resolve_system_bindings(worker_roots: &[PathBuf]) -> Vec<String> {
    let mut resolved: Vec<String> = Vec::new();
    for root in worker_roots {
        if !root.is_absolute() {
            continue;
        }
        if let Ok(canonical) = std::fs::canonicalize(root) {
            push_unique(&mut resolved, path_text(&canonical));
        }
    }
    for candidate in OPTIONAL_SYSTEM_DIRECTORIES {
        let path = Path::new(candidate);
        if path.is_dir() {
            push_unique(&mut resolved, (*candidate).to_owned());
        }
    }
    for candidate in OPTIONAL_SYSTEM_FILES {
        let path = Path::new(candidate);
        if path.is_file() {
            push_unique(&mut resolved, (*candidate).to_owned());
        }
    }
    resolved
}

/// Build a strict specification after gating on every required control.
pub fn build_sandbox_spec(
    controls: &[String],
    root: &Path,
    workspace: &Path,
    worker_roots: &[PathBuf],
    scratch_bytes: i64,
) -> Result<SandboxSpec, SandboxSetupError> {
    let controls: BTreeSet<String> = controls.iter().cloned().collect();
    let missing: Vec<&str> = STRICT_REQUIRED_CONTROLS
        .iter()
        .copied()
        .filter(|control| !controls.contains(*control))
        .collect();
    if !missing.is_empty() {
        return Err(SandboxSetupError::new(format!(
            "the strict profile requires controls that were not probed: {}",
            missing.join(", ")
        )));
    }
    if !safe_host_path(root) || !safe_host_path(workspace) {
        return Err(SandboxSetupError::new(
            "sandbox paths must be absolute and free of traversal segments",
        ));
    }
    if path_text(root).trim_end_matches('/').is_empty()
        || path_text(workspace).trim_end_matches('/').is_empty()
    {
        return Err(SandboxSetupError::new(
            "the sandbox root and workspace must not be the host root",
        ));
    }
    let workspace_resolved =
        std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    Ok(SandboxSpec {
        controls,
        root: root.to_path_buf(),
        workspace: workspace.to_path_buf(),
        readonly_bindings: resolve_system_bindings(worker_roots),
        writable_bindings: vec![path_text(&workspace_resolved)],
        device_bindings: DEVICE_NODES
            .iter()
            .filter(|candidate| Path::new(candidate).exists())
            .map(|candidate| (*candidate).to_owned())
            .collect(),
        scratch_paths: SCRATCH_PATHS
            .iter()
            .map(|path| (*path).to_owned())
            .collect(),
        scratch_bytes,
    })
}

/// The portable controls this native layer actually enforces.
///
/// This list intentionally omits `worker_concurrency_cap`, artifact digest and
/// classification controls: those belong to the future tools executor. It also
/// never claims filesystem confinement for the portable profile.
pub fn portable_controls(limits: &ProcessLimits) -> Vec<&'static str> {
    let _ = limits;
    let mut controls = vec![
        "bootstrap_deadline",
        "cwd_confined",
        "environment_minimal",
        "output_size_limit",
        "process_isolation",
        "stdin_backpressure_bound",
        "termination_deadline",
        "wall_clock_timeout",
    ];
    #[cfg(unix)]
    {
        controls.push("file_descriptor_close");
        controls.push("process_group_kill");
        if limits.enforce_resource_limits {
            controls.push("resource_limits");
        }
    }
    #[cfg(windows)]
    {
        controls.push("explicit_handle_inheritance");
        controls.push("job_object_containment");
        controls.push("kill_on_job_close");
        controls.push("process_tree_containment");
        controls.push("suspended_assign_before_resume");
    }
    controls.sort_unstable();
    controls
}

/// The controls a profile can honestly advertise on this host.
///
/// Strict controls are added only when the probe actually verified them.
pub fn enforced_controls(
    profile: IsolationProfile,
    probe: Option<&SandboxProbe>,
    limits: &ProcessLimits,
) -> Vec<String> {
    let mut controls: BTreeSet<String> = portable_controls(limits)
        .into_iter()
        .map(str::to_owned)
        .collect();
    if profile == IsolationProfile::StrictLinux {
        if let Some(probe) = probe.filter(|probe| probe.available) {
            for control in &probe.controls {
                if control != "pid_namespace"
                    && (STRICT_REQUIRED_CONTROLS.contains(&control.as_str())
                        || STRICT_OPTIONAL_CONTROLS.contains(&control.as_str()))
                {
                    controls.insert(control.clone());
                }
            }
        }
    }
    controls.into_iter().collect()
}

/// Decide whether the configured profile can back a side-effecting tool.
pub fn judge_sandbox(profile: IsolationProfile, probe: Option<&SandboxProbe>) -> SandboxVerdict {
    if profile != IsolationProfile::StrictLinux {
        return SandboxVerdict {
            allowed: true,
            reason: "the portable profile confines a process, not the filesystem".to_owned(),
            controls: Vec::new(),
        };
    }
    match probe {
        Some(probe) if probe.available => SandboxVerdict {
            allowed: true,
            reason: String::new(),
            controls: probe.controls.clone(),
        },
        _ => {
            let detail = probe.map(|probe| probe.detail.as_str()).unwrap_or("");
            SandboxVerdict {
                allowed: false,
                reason: if detail.is_empty() {
                    "the strict profile is not available on this host, so side effects are refused"
                        .to_owned()
                } else {
                    format!(
                        "the strict profile is not available on this host, so side effects are \
                         refused: {detail}"
                    )
                },
                controls: Vec::new(),
            }
        }
    }
}

/// Establish the requested controls in the calling process.
///
/// This is only safe in a freshly executed, single-threaded helper (the probe
/// helper). Production workers use the fork/exec prepared plan in the Unix
/// launcher so that multithreaded parents never run sandbox setup logic in a
/// `pre_exec` context.
pub fn isolate_process(spec: &SandboxSpec) -> Result<Vec<String>, SandboxSetupError> {
    #[cfg(target_os = "linux")]
    {
        crate::sandbox_plan::isolate_in_place(spec)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = spec;
        Err(SandboxSetupError::new(
            "the strict profile requires Linux namespaces",
        ))
    }
}

/// Verify the requested controls from inside an established jail.
///
/// `established` is the list returned by [`isolate_process`]. Namespace
/// controls are accepted only when the setup actually observed a namespace
/// inode change, not merely because `unshare` returned success.
pub fn verify_controls(
    spec: &SandboxSpec,
    canary: &str,
    workspace: &str,
    established: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut seen: BTreeSet<&str> = STRICT_REQUIRED_CONTROLS
        .iter()
        .chain(STRICT_OPTIONAL_CONTROLS.iter())
        .copied()
        .collect();
    seen.extend(spec.controls.iter().map(String::as_str));
    let mut verified = Vec::new();
    let mut failed = Vec::new();
    for name in seen {
        if !spec.controls.contains(name) {
            continue;
        }
        let holds = match name {
            "chroot_workspace_only" => canary_is_invisible(canary),
            "network_isolation" => network_is_isolated(),
            "workspace_read_write" => workspace_is_writable(workspace),
            "readonly_system_bindings" => system_bindings_are_readonly(),
            "pid_namespace" => std::process::id() == 1,
            "ipc_namespace" | "uts_namespace" | "user_namespace" | "mount_namespace" => {
                established.iter().any(|value| value == name)
            }
            "procfs" => procfs_is_namespaced(),
            "device_nodes_minimal" => {
                device_list_is_minimal() && devices_are_bound(&spec.device_bindings)
            }
            "scratch_tmpfs" => scratch_is_writable(&spec.scratch_paths),
            _ => false,
        };
        if holds {
            verified.push(name.to_owned());
        } else {
            failed.push(name.to_owned());
        }
    }
    verified.sort();
    failed.sort();
    (verified, failed)
}

/// Build the jail in this process and report what really worked.
///
/// The probe helper calls this after `exec`; it is not a product worker path.
pub fn run_probe(spec_payload: &str, canary_payload: &str) -> i32 {
    let (spec, canary) = match probe_entry(spec_payload, canary_payload) {
        Ok(entry) => entry,
        Err(error) => {
            emit_probe_result(&[], &truncate_detail(error.detail()));
            return PROBE_FAILURE_EXIT;
        }
    };
    let established = match isolate_process(&spec) {
        Ok(established) => established,
        Err(error) => {
            emit_probe_result(&[], &truncate_detail(error.detail()));
            return PROBE_FAILURE_EXIT;
        }
    };
    let (verified, _failed) =
        verify_controls(&spec, &canary, &path_text(&spec.workspace), &established);
    if !STRICT_REQUIRED_CONTROLS
        .iter()
        .all(|control| verified.iter().any(|value| value == control))
    {
        return PROBE_FAILURE_EXIT;
    }
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
    emit_probe_result(&verified, &truncate_detail(&detail));
    0
}

/// Options for [`probe_sandbox`].
#[derive(Debug, Clone)]
pub struct SandboxProbeOptions {
    /// Path to the native `kala-process-probe` helper.
    pub helper: PathBuf,
    /// Host roots the worker needs read-only (the crate itself, native worker
    /// directory, and so on).
    pub worker_roots: Vec<PathBuf>,
    /// Scratch tmpfs size to request.
    pub scratch_bytes: i64,
    /// Probe wall-clock bound.
    pub timeout: Duration,
}

impl SandboxProbeOptions {
    /// Construct options with the compatibility scratch size and probe timeout.
    pub fn new(helper: impl Into<PathBuf>) -> Self {
        Self {
            helper: helper.into(),
            worker_roots: Vec::new(),
            scratch_bytes: MIN_DEVICE_BYTES,
            timeout: Duration::from_secs_f64(PROBE_TIMEOUT_SECONDS),
        }
    }

    /// Discover the helper next to the current executable, or through
    /// `KALA_PROCESS_PROBE`.
    pub fn discover() -> Option<Self> {
        if let Some(path) = std::env::var_os("KALA_PROCESS_PROBE") {
            return Some(Self::new(path));
        }
        let current = std::env::current_exe().ok()?;
        let directory = current.parent()?;
        for candidate in [
            directory.join(exe_name("kala-process-probe")),
            directory
                .parent()
                .unwrap_or(directory)
                .join(exe_name("kala-process-probe")),
        ] {
            if candidate.is_file() {
                return Some(Self::new(candidate));
            }
        }
        None
    }
}

/// Probe whether the strict Linux profile can be established on this host.
///
/// The helper is a native executable shipped by this crate. It builds exactly
/// the jail a worker would build and verifies, from the inside, namespace
/// changes, filesystem invisibility, network unreachability, workspace
/// writability and read-only system bindings.
pub fn probe_sandbox(options: &SandboxProbeOptions) -> SandboxProbe {
    if !cfg!(target_os = "linux") {
        return unavailable("the strict profile requires Linux namespaces");
    }
    let helper = match std::fs::canonicalize(&options.helper) {
        Ok(helper) if helper.is_file() => helper,
        _ => return unavailable("no usable native probe helper is available"),
    };
    let started = Instant::now();
    let scratch = match tempfile::Builder::new()
        .prefix("kala-sandbox-probe-")
        .tempdir()
    {
        Ok(directory) => directory,
        Err(_) => return unavailable("the sandbox probe could not create a scratch directory"),
    };
    let root = scratch.path().join("root");
    let workspace = scratch.path().join("workspace");
    if std::fs::create_dir(&workspace).is_err() {
        return unavailable("the sandbox probe could not create its workspace");
    }
    let canary = scratch.path().join("canary");
    if std::fs::write(&canary, CANARY_PAYLOAD).is_err() {
        return unavailable("the sandbox probe could not create its canary");
    }
    let controls: Vec<String> = STRICT_REQUIRED_CONTROLS
        .iter()
        .chain(
            STRICT_OPTIONAL_CONTROLS
                .iter()
                .filter(|control| **control != "pid_namespace"),
        )
        .map(|control| (*control).to_owned())
        .collect();
    let spec = match build_sandbox_spec(
        &controls,
        &root,
        &workspace,
        &options.worker_roots,
        options.scratch_bytes,
    ) {
        Ok(spec) => spec,
        Err(error) => {
            return unavailable(&format!(
                "the sandbox probe could not build a specification: {}",
                error.detail()
            ));
        }
    };
    if std::fs::create_dir_all(&root).is_err() {
        return unavailable("the sandbox probe could not create its jail root");
    }
    let environment = match crate::ChildEnvironment::minimal(&workspace) {
        Ok(environment) => environment,
        Err(_) => return unavailable("the sandbox probe could not build its environment"),
    };
    let limits = ProcessLimits {
        wall_timeout: options.timeout,
        bootstrap_timeout: options.timeout.min(Duration::from_secs(5)),
        termination_timeout: Duration::from_secs(5),
        ..ProcessLimits::default()
    };
    let mut process = match crate::ProcessSpec::new(&helper, &workspace) {
        Ok(process) => process,
        Err(_) => {
            return unavailable("the sandbox probe could not build its process specification");
        }
    };
    process = process
        .with_environment(environment)
        .with_limits(limits)
        .arg(spec.to_payload().to_string())
        .arg(json!(canary.to_string_lossy()).to_string());
    let token = crate::CancelToken::new();
    let outcome = crate::supervise(&process, &[], &token);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            let detail = format!("the sandbox probe could not start: {}", error);
            return unavailable_with_elapsed(&detail, elapsed_ms);
        }
    };
    if outcome.end != crate::ProcessEnd::Exited(crate::ExitStatus::exited(0)) {
        let detail = format!(
            "the strict profile is unavailable on this host ({:?}{})",
            outcome.end,
            if outcome.stderr_summary.is_empty() {
                String::new()
            } else {
                format!(": {}", outcome.stderr_summary)
            }
        );
        let detail = if outcome.stdout.is_empty() {
            detail
        } else {
            format!(
                "{detail}; probe output: {}",
                outcome.stdout.to_string_lossy()
            )
        };
        return unavailable_with_elapsed(&detail, elapsed_ms);
    }
    let stdout = match outcome.stdout_str() {
        Ok(text) => text.trim(),
        Err(_) => {
            return unavailable_with_elapsed("the sandbox probe returned no result", elapsed_ms);
        }
    };
    let payload = if stdout.is_empty() {
        json!({})
    } else {
        match serde_json::from_str::<Value>(stdout) {
            Ok(value) => value,
            Err(_) => {
                return unavailable_with_elapsed(
                    "the sandbox probe returned no result",
                    elapsed_ms,
                );
            }
        }
    };
    let known: BTreeSet<&str> = STRICT_REQUIRED_CONTROLS
        .iter()
        .chain(STRICT_OPTIONAL_CONTROLS.iter())
        .copied()
        .collect();
    let controls: Vec<String> = payload
        .get("controls")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| known.contains(name) && *name != "pid_namespace")
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if !STRICT_REQUIRED_CONTROLS
        .iter()
        .all(|control| controls.iter().any(|value| value == control))
    {
        return unavailable_with_elapsed("a required sandbox control was not verified", elapsed_ms);
    }
    let mut controls = controls;
    controls.sort();
    controls.dedup();
    SandboxProbe {
        available: true,
        platform: platform_name(),
        controls,
        detail: String::new(),
        elapsed_ms,
    }
}

/// Redact and bound a child's stderr exactly like
/// `kala_agent.tools.protocol.sanitize_child_stderr`.
pub fn sanitize_child_stderr(payload: &[u8], replacement: Option<&str>) -> String {
    if payload.is_empty() {
        return String::new();
    }
    let sample = &payload[..payload.len().min(MAX_SANITIZED_DETAIL * 2)];
    let mut text = String::from_utf8_lossy(sample).into_owned();
    text.retain(|character| character == '\n' || character >= ' ');
    if let Some(replacement) = replacement.filter(|value| !value.is_empty()) {
        text = text.replace(replacement, "<workspace>");
    }
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    let selected: Vec<&str> = if lines.len() > 2 {
        vec![lines[0].as_str(), "...", lines[lines.len() - 1].as_str()]
    } else {
        lines.iter().map(String::as_str).collect()
    };
    selected
        .join(" | ")
        .chars()
        .take(MAX_SANITIZED_DETAIL)
        .collect()
}

const MAX_SANITIZED_DETAIL: usize = 512;

/// True when a canary outside the jail cannot be read.
pub fn canary_is_invisible(canary: &str) -> bool {
    std::fs::read(canary).is_err()
}

/// True when the workspace accepts a write probe and cleans it up.
pub fn workspace_is_writable(workspace: &str) -> bool {
    let probe = Path::new(workspace).join(".sandbox-write-probe");
    if std::fs::write(&probe, CANARY_PAYLOAD).is_err() {
        return false;
    }
    let _ = std::fs::remove_file(&probe);
    true
}

/// True when `/usr` exists but refuses a write.
pub fn system_bindings_are_readonly() -> bool {
    let system = Path::new("/usr");
    if !system.is_dir() {
        return false;
    }
    let probe = system.join(".sandbox-write-probe");
    if std::fs::write(&probe, b"").is_err() {
        return true;
    }
    let _ = std::fs::remove_file(&probe);
    false
}

/// True when `/proc/1/stat` is readable inside the jail.
pub fn procfs_is_namespaced() -> bool {
    std::fs::read_to_string("/proc/1/stat")
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
}

/// True when every scratch candidate is writable.
pub fn scratch_is_writable(candidates: &[String]) -> bool {
    for candidate in candidates {
        let probe = Path::new(candidate).join(".sandbox-scratch-probe");
        if std::fs::write(&probe, b"").is_err() {
            return false;
        }
        let _ = std::fs::remove_file(&probe);
    }
    true
}

/// True when every requested device path is still a character device inside
/// the jail. A failed bind leaves the regular mountpoint file behind, so this
/// distinguishes a real device binding from that silent failure.
pub fn devices_are_bound(devices: &[String]) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        !devices.is_empty()
            && devices.iter().all(|device| {
                std::fs::metadata(device)
                    .map(|metadata| metadata.file_type().is_char_device())
                    .unwrap_or(false)
            })
    }
    #[cfg(not(unix))]
    {
        !devices.is_empty()
    }
}

/// True when `/proc/net/dev` lists only loopback (or is unreadable).
pub fn device_list_is_minimal() -> bool {
    let listing = match std::fs::read_to_string("/proc/net/dev") {
        Ok(listing) => listing,
        Err(_) => return true,
    };
    let names: BTreeSet<&str> = listing
        .lines()
        .skip(2)
        .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim()))
        .collect();
    names.is_subset(&BTreeSet::from(["lo"]))
}

/// True when an external TCP connect is refused or unreachable from the jail.
pub fn network_is_isolated() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux_network_is_isolated()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Drop scratch paths that would hide a required binding.
pub fn unshadowed_scratch_paths(spec: &SandboxSpec) -> Vec<String> {
    let reserved: Vec<&String> = spec
        .readonly_bindings
        .iter()
        .chain(spec.writable_bindings.iter())
        .collect();
    spec.scratch_paths
        .iter()
        .filter(|candidate| {
            !reserved.iter().any(|binding| {
                binding.as_str() == candidate.as_str()
                    || binding.starts_with(&format!("{candidate}/"))
            })
        })
        .cloned()
        .collect()
}

fn probe_entry(
    spec_payload: &str,
    canary_payload: &str,
) -> Result<(SandboxSpec, String), SandboxProbeError> {
    let decoded: Value = serde_json::from_str(spec_payload)
        .map_err(|_| SandboxProbeError::new("the probe payload is malformed"))?;
    let spec = SandboxSpec::from_payload(&decoded)
        .map_err(|_| SandboxProbeError::new("the probe payload is malformed"))?;
    let canary: Value = serde_json::from_str(canary_payload)
        .map_err(|_| SandboxProbeError::new("the probe payload is malformed"))?;
    let canary = canary
        .as_str()
        .ok_or_else(|| SandboxProbeError::new("the probe payload is malformed"))?
        .to_owned();
    Ok((spec, canary))
}

fn emit_probe_result(controls: &[String], detail: &str) {
    let payload = json!({
        "controls": controls,
        "detail": detail,
    });
    println!("{payload}");
}

fn unavailable(reason: &str) -> SandboxProbe {
    unavailable_with_elapsed(reason, 0.0)
}

fn unavailable_with_elapsed(reason: &str, elapsed_ms: f64) -> SandboxProbe {
    SandboxProbe {
        available: false,
        platform: platform_name(),
        controls: Vec::new(),
        detail: truncate_detail(reason),
        elapsed_ms,
    }
}

fn truncate_detail(detail: &str) -> String {
    detail.chars().take(MAX_PROBE_DETAIL).collect()
}

fn round_three(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let scaled = value * 1_000.0;
    let floor = scaled.floor();
    let fraction = scaled - floor;
    let rounded = match fraction.partial_cmp(&0.5) {
        Some(std::cmp::Ordering::Greater) => floor + 1.0,
        Some(std::cmp::Ordering::Less) => floor,
        _ => {
            if (floor as i64) % 2 == 0 {
                floor
            } else {
                floor + 1.0
            }
        }
    };
    rounded / 1_000.0
}

fn path_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}

fn safe_host_path(path: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
}

fn path_value(value: Option<&Value>) -> Option<PathBuf> {
    value.and_then(Value::as_str).map(PathBuf::from)
}

fn string_sequence(value: Option<&Value>) -> Option<Vec<String>> {
    value?
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

#[cfg(target_os = "linux")]
fn linux_network_is_isolated() -> bool {
    use std::os::fd::RawFd;
    // SAFETY: plain BSD socket calls with no aliasing; the descriptor is
    // closed on every path. A non-blocking external connect in a fresh network
    // namespace fails immediately with ENETUNREACH/EPERM.
    unsafe {
        let handle: RawFd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_NONBLOCK, 0);
        if handle < 0 {
            return false;
        }
        let mut address: libc::sockaddr_in = std::mem::zeroed();
        address.sin_family = libc::AF_INET as libc::sa_family_t;
        address.sin_port = 53u16.to_be();
        address.sin_addr.s_addr = u32::from_ne_bytes([8, 8, 8, 8]);
        let result = libc::connect(
            handle,
            (&address as *const libc::sockaddr_in).cast::<libc::sockaddr>(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        let error = io::Error::last_os_error().raw_os_error();
        libc::close(handle);
        if result == 0 {
            return false;
        }
        !matches!(
            error,
            Some(libc::EINPROGRESS) | Some(libc::EALREADY) | Some(libc::ETIMEDOUT)
        )
    }
}
