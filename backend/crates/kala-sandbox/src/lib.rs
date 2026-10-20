//! The Kala sandbox: the strict Linux jail, the probe that decides whether
//! the host can enforce it, and the refusal policy that guards side effects.
//!
//! This crate is a complete port of `kala_agent.tools.sandbox`. Every public
//! function, every validation message, the error precedence, the payload
//! bytes and the limits are reproduced. The differences are listed in
//! `REPORT.md` and in the crate README; the two that matter most are:
//!
//! * The probe child is a **native** helper (`kala-sandbox-probe`), not a
//!   Python interpreter. The parent launches it through the accepted
//!   [`kala_process`] supervisor, so the launch, the wall-clock deadline, the
//!   cancellation token and the output bounds are the ones the rest of the
//!   backend already uses. No interpreter is started at product time.
//! * The **production worker jail** is built by the native init helper
//!   (`kala-sandbox-init`), which the accepted [`kala_process`] supervisor
//!   launches instead of the worker. The helper performs the PID-1
//!   transition the accepted launcher does not have: the worker runs in a PID
//!   namespace whose init is PID 1, so the worker cannot see the host process
//!   table and the whole namespace dies with the jail. See [`WorkerJail`].
//!
//! # Guarantees
//!
//! * The strict profile is advertised only after a throwaway child actually
//!   built the namespace/mount/chroot jail and verified, from the inside, that
//!   the network is unreachable, the host filesystem is invisible and the
//!   workspace is writable.
//! * A jail that cannot be established fails closed: the caller receives a
//!   [`SandboxSetupError`] and the worker is never executed.
//! * The refusal reasons of the strict profile are the Python reasons, byte
//!   for byte, so a decision recorded before the migration still explains
//!   itself after it.
//!
//! # Platform
//!
//! The strict profile is Linux only. On every other platform
//! [`probe_sandbox`] reports the profile as unavailable and
//! [`judge_sandbox`] refuses it; the portable profile is unaffected and keeps
//! its explicit "confines a process, not the filesystem" wording.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod constants;
mod error;
mod isolation;
mod json;
mod parse;
mod path;
mod probe;
mod probe_run;
mod profile;
mod resources;
mod spec;
mod worker;

pub use constants::{
    CANARY_PAYLOAD, CLONE_NEWIPC, CLONE_NEWNET, CLONE_NEWNS, CLONE_NEWPID, CLONE_NEWUSER,
    CLONE_NEWUTS, DEVICE_NODES, ISOLATION_FAILURE_EXIT, MAX_PROBE_DETAIL, MIN_DEVICE_BYTES,
    MS_BIND, MS_NODEV, MS_NOEXEC, MS_NOSUID, MS_PRIVATE, MS_RDONLY, MS_REC, MS_REMOUNT,
    NAMESPACE_CONTROLS, OPTIONAL_SYSTEM_DIRECTORIES, OPTIONAL_SYSTEM_FILES, PROBE_FAILURE_EXIT,
    PROBE_TIMEOUT_SECONDS, SCRATCH_PATHS, STRICT_OPTIONAL_CONTROLS, STRICT_REQUIRED_CONTROLS,
    UNSHARE_ORDER, all_controls, is_known_control, namespace_flag,
};
pub use error::{FailureClass, SandboxError, SandboxProbeError, SandboxSetupError};
pub use isolation::{
    PidRole, bind_device, bind_read_write, bind_readonly, build_jail, child_pre_exec,
    child_pre_exec_or_exit, close_inherited_fds, enter_pid_namespace,
    enter_pid_namespace_with_role, exit_code_from_status, isolate_process, libc_error,
    make_mountpoint, mount, mount_scratch, mount_scratch_set, os_reason, start_recording,
    take_recorded_calls, unshadowed_scratch_paths, unshare, unshare_namespaces,
};
pub use json::{JsonValue, dump_float, dump_string_list, escape_text, round_three};
pub use parse::{JsonParseError, parse_json};
pub use path::{native_text, posix_text, resolve_strict_false};
pub use probe::{
    SandboxProbe, SandboxVerdict, canary_is_invisible, control_is_known, device_list_is_minimal,
    judge_sandbox, network_is_isolated, procfs_is_namespaced, scratch_is_writable,
    system_bindings_are_readonly, truncate_detail, unavailable_probe, verify_controls,
    verify_controls_with, workspace_is_writable,
};
pub use probe_run::{
    ProbeOptions, discover_helper, emit_probe_result, judge, probe_entry, probe_paths,
    probe_sandbox, run_probe,
};
pub use profile::IsolationProfile;
pub use resources::{
    InterpreterLayout, child_resource_paths, realpath, resolve_system_bindings,
    resolve_system_bindings_with,
};
pub use spec::{
    SandboxSpec, build_sandbox_spec, build_sandbox_spec_with, integer_value, python_int,
    python_path,
};
pub use worker::{
    InitRefusal, InitRequest, WorkerCommand, WorkerJail, arm_parent_death_signal,
    parse_init_arguments, run_init, run_worker, run_worker_in_jail,
};

/// The host platform description.
///
/// The Python module reports `f"{sys.platform}-{os.uname().machine}"` on
/// Linux and the bare `sys.platform` elsewhere.
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

/// The controls the native production path can enforce.
///
/// Every control the probe can verify is enforceable: the production path runs
/// the worker under the native init helper, which performs the PID-1
/// transition the accepted launcher does not have.
pub fn native_supported_controls() -> Vec<&'static str> {
    STRICT_OPTIONAL_CONTROLS.to_vec()
}

/// The controls the accepted `kala-process` launcher can enforce on its own.
///
/// The launcher builds the jail after `fork` and before `execve` and has no
/// PID-1 transition, so it cannot enforce `pid_namespace`.
pub fn launcher_supported_controls() -> Vec<&'static str> {
    STRICT_OPTIONAL_CONTROLS
        .iter()
        .copied()
        .filter(|control| *control != "pid_namespace")
        .collect()
}

/// Adapt a specification to the accepted native launcher.
///
/// The accepted [`kala_process`] launcher builds the jail after `fork` and
/// before `execve`, without a PID-1 transition. A specification that requests
/// `pid_namespace` is therefore **refused**: the launcher would silently
/// build a weaker jail than the caller asked for. The production worker jail
/// uses the native init helper instead; see [`WorkerJail`].
///
/// The conversion never widens authority: a control that the launcher cannot
/// establish is refused instead of being claimed.
pub fn native_sandbox_spec(
    spec: &SandboxSpec,
) -> Result<kala_process::SandboxSpec, SandboxSetupError> {
    if spec.controls.contains("pid_namespace") {
        return Err(SandboxSetupError::new(
            "the accepted launcher cannot enforce the pid_namespace control; \
             use the native init helper for the production worker jail",
        ));
    }
    let controls: std::collections::BTreeSet<String> = spec
        .controls
        .iter()
        .filter(|control| {
            STRICT_REQUIRED_CONTROLS.contains(&control.as_str())
                || launcher_supported_controls().contains(&control.as_str())
        })
        .cloned()
        .collect();
    Ok(kala_process::SandboxSpec {
        controls,
        root: spec.root.clone(),
        workspace: spec.workspace.clone(),
        readonly_bindings: spec.readonly_bindings.clone(),
        writable_bindings: spec.writable_bindings.clone(),
        device_bindings: spec.device_bindings.clone(),
        scratch_paths: spec.scratch_paths.clone(),
        scratch_bytes: spec.scratch_bytes,
    })
}
