//! The probe outcome, the refusal verdict and the control verification.

use std::path::Path;

use crate::constants::{
    SCRATCH_PATHS, STRICT_OPTIONAL_CONTROLS, STRICT_REQUIRED_CONTROLS, is_known_control,
};
use crate::json::{JsonValue, round_three};
use crate::profile::IsolationProfile;
use crate::spec::SandboxSpec;

/// The outcome of a preflight probe of the strict profile.
///
/// `controls` only ever contains controls that the probe established and
/// verified inside a child process, so a profile advertises exactly what this
/// host actually enforces.
#[derive(Debug, Clone, PartialEq)]
pub struct SandboxProbe {
    /// Whether every required control was verified.
    pub available: bool,
    /// The host platform description.
    pub platform: String,
    /// The verified controls, sorted.
    pub controls: Vec<String>,
    /// A short, sanitised reason when the probe is unavailable.
    pub detail: String,
    /// The probe duration in milliseconds.
    pub elapsed_ms: f64,
}

impl SandboxProbe {
    /// A probe that reports the profile as unavailable.
    pub fn unavailable(reason: &str, elapsed_ms: f64) -> Self {
        Self {
            available: false,
            platform: crate::platform_name(),
            controls: Vec::new(),
            detail: truncate_detail(reason),
            elapsed_ms,
        }
    }

    /// The exact Python payload shape, with the Python key order.
    pub fn to_payload(&self) -> JsonValue {
        JsonValue::object(vec![
            ("available".to_owned(), JsonValue::Bool(self.available)),
            (
                "controls".to_owned(),
                JsonValue::Array(
                    self.controls
                        .iter()
                        .map(|control| JsonValue::Text(control.clone()))
                        .collect(),
                ),
            ),
            ("detail".to_owned(), JsonValue::Text(self.detail.clone())),
            (
                "elapsed_ms".to_owned(),
                JsonValue::Float(round_three(self.elapsed_ms)),
            ),
            (
                "platform".to_owned(),
                JsonValue::Text(self.platform.clone()),
            ),
        ])
    }

    /// The payload rendered exactly like `json.dumps(payload, ensure_ascii=True)`.
    pub fn to_json(&self) -> String {
        self.to_payload().dump()
    }
}

/// The result of asking whether a side effect may proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxVerdict {
    /// Whether the configured profile can back the side effect.
    pub allowed: bool,
    /// A sanitised reason.
    pub reason: String,
    /// The verified controls that back an allowed strict verdict.
    pub controls: Vec<String>,
}

impl SandboxVerdict {
    /// The exact Python payload shape, with the Python key order.
    pub fn to_payload(&self) -> JsonValue {
        JsonValue::object(vec![
            ("allowed".to_owned(), JsonValue::Bool(self.allowed)),
            (
                "controls".to_owned(),
                JsonValue::Array(
                    self.controls
                        .iter()
                        .map(|control| JsonValue::Text(control.clone()))
                        .collect(),
                ),
            ),
            ("reason".to_owned(), JsonValue::Text(self.reason.clone())),
        ])
    }

    /// The payload rendered exactly like `json.dumps(payload, ensure_ascii=True)`.
    pub fn to_json(&self) -> String {
        self.to_payload().dump()
    }
}

/// Decide whether the configured profile can back a side-effecting tool.
///
/// The refusal text is the Python text, byte for byte.
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

/// The controls that hold inside the jail and the ones that do not.
///
/// This is the exact Python rule: a namespace control holds when it was
/// requested, because [`crate::isolate_process`] raises when the matching
/// `unshare` fails.
pub fn verify_controls(
    spec: &SandboxSpec,
    canary: &str,
    workspace: &str,
) -> (Vec<String>, Vec<String>) {
    verify_controls_with(spec, canary, workspace, None)
}

/// The control verification that also requires the controls to have been
/// established by [`crate::isolate_process`].
///
/// The probe helper uses this variant so a namespace is never reported as
/// verified unless the jail actually created it.
pub fn verify_controls_with(
    spec: &SandboxSpec,
    canary: &str,
    workspace: &str,
    established: Option<&[String]>,
) -> (Vec<String>, Vec<String>) {
    let mut names: Vec<&str> = STRICT_REQUIRED_CONTROLS
        .iter()
        .chain(STRICT_OPTIONAL_CONTROLS.iter())
        .copied()
        .collect();
    names.sort_unstable();
    names.dedup();
    let mut verified: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for name in names {
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
                match established {
                    None => true,
                    Some(established) => established.iter().any(|value| value == name),
                }
            }
            "procfs" => procfs_is_namespaced(),
            "device_nodes_minimal" => device_list_is_minimal(),
            "scratch_tmpfs" => scratch_is_writable(SCRATCH_PATHS),
            _ => false,
        };
        if holds {
            verified.push(name.to_owned());
        } else {
            failed.push(name.to_owned());
        }
    }
    (verified, failed)
}

/// True when a canary outside the jail cannot be read.
pub fn canary_is_invisible(canary: &str) -> bool {
    std::fs::read(canary).is_err()
}

/// True when the workspace accepts a write probe and cleans it up.
pub fn workspace_is_writable(workspace: &str) -> bool {
    let probe = Path::new(workspace).join(".sandbox-write-probe");
    if std::fs::write(&probe, crate::constants::CANARY_PAYLOAD).is_err() {
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
///
/// The Python helper verifies the default scratch paths, not the paths of the
/// specification that is being probed.
pub fn scratch_is_writable(candidates: &[&str]) -> bool {
    for candidate in candidates {
        let probe = Path::new(candidate).join(".sandbox-scratch-probe");
        if std::fs::write(&probe, b"").is_err() {
            return false;
        }
        let _ = std::fs::remove_file(&probe);
    }
    true
}

/// True when `/proc/net/dev` lists only loopback.
pub fn device_list_is_minimal() -> bool {
    let listing = match std::fs::read_to_string("/proc/net/dev") {
        Ok(listing) => listing,
        Err(_) => return true,
    };
    let names: Vec<String> = listing
        .lines()
        .skip(2)
        .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim().to_owned()))
        .collect();
    names.iter().all(|name| name == "lo")
}

/// True when an external TCP connect is refused or unreachable from the jail.
///
/// The socket is non-blocking: a blocked connect would stall the probe, while
/// an in-flight connect proves that the network is reachable.
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

/// Truncate a probe detail to the Python bound.
pub fn truncate_detail(detail: &str) -> String {
    detail
        .chars()
        .take(crate::constants::MAX_PROBE_DETAIL)
        .collect()
}

/// Whether a control name is part of the closed control table.
pub fn control_is_known(control: &str) -> bool {
    is_known_control(control)
}

/// Build the refusal text of an unavailable probe.
pub fn unavailable_probe(reason: &str, elapsed_ms: f64) -> SandboxProbe {
    SandboxProbe::unavailable(reason, elapsed_ms)
}

#[cfg(target_os = "linux")]
fn linux_network_is_isolated() -> bool {
    use std::os::fd::RawFd;
    // SAFETY: plain BSD socket calls with no aliasing; the descriptor is
    // closed on every path.
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
        let error = std::io::Error::last_os_error().raw_os_error();
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
