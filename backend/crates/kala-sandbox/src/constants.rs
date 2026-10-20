//! Constants shared with `kala_agent.tools.sandbox`.
//!
//! Every value here is the exact Python constant. The control tables are the
//! contract that a jail must satisfy before a side-effecting tool may run.

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

/// Exit code the sandbox uses to signal that isolation failed.
pub const ISOLATION_FAILURE_EXIT: i32 = 121;
/// Exit code the probe uses for a failed probe.
pub const PROBE_FAILURE_EXIT: i32 = 122;
/// Probe wall-clock bound, in seconds.
pub const PROBE_TIMEOUT_SECONDS: f64 = 30.0;
/// Maximum number of characters kept in a probe detail.
pub const MAX_PROBE_DETAIL: usize = 480;
/// Payload written to the canary file that must stay invisible inside the jail.
pub const CANARY_PAYLOAD: &[u8] = b"kala-sandbox-canary";
/// Default scratch tmpfs size, in bytes.
pub const MIN_DEVICE_BYTES: i64 = 1 << 20;

/// Controls a strict Linux jail must establish before it is advertised.
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
/// `pid_namespace` is listed because the Python source lists it. The native
/// launcher in `kala-process` has no PID-1 double-fork transition, so a
/// specification that is handed to that launcher must not claim the control.
pub const STRICT_OPTIONAL_CONTROLS: &[&str] = &[
    "device_nodes_minimal",
    "ipc_namespace",
    "pid_namespace",
    "procfs",
    "scratch_tmpfs",
    "uts_namespace",
];

/// The order in which namespaces are unshared.
///
/// The order is part of the contract: the user namespace must come first so
/// the remaining namespaces can be created with the mapped identity.
pub const UNSHARE_ORDER: &[&str] = &[
    "user_namespace",
    "network_isolation",
    "mount_namespace",
    "ipc_namespace",
    "uts_namespace",
];

/// The `unshare` flag for each namespace control.
pub const NAMESPACE_CONTROLS: &[(&str, i32)] = &[
    ("user_namespace", CLONE_NEWUSER),
    ("network_isolation", CLONE_NEWNET),
    ("mount_namespace", CLONE_NEWNS),
    ("ipc_namespace", CLONE_NEWIPC),
    ("uts_namespace", CLONE_NEWUTS),
];

/// Device nodes a minimal jail may expose.
pub const DEVICE_NODES: &[&str] = &[
    "/dev/full",
    "/dev/null",
    "/dev/random",
    "/dev/urandom",
    "/dev/zero",
];

/// Paths that receive a private tmpfs.
pub const SCRATCH_PATHS: &[&str] = &["/dev/shm", "/tmp"];

/// Optional host directories a jailed worker may reach read-only.
pub const OPTIONAL_SYSTEM_DIRECTORIES: &[&str] =
    &["/bin", "/lib", "/lib32", "/lib64", "/sbin", "/usr"];

/// Optional host files a jailed worker may reach read-only.
pub const OPTIONAL_SYSTEM_FILES: &[&str] =
    &["/etc/ld.so.cache", "/etc/localtime", "/etc/nsswitch.conf"];

/// The `unshare` flag for a namespace control.
pub fn namespace_flag(control: &str) -> Option<i32> {
    NAMESPACE_CONTROLS
        .iter()
        .find(|(name, _)| *name == control)
        .map(|(_, flag)| *flag)
}

/// Every control name the module knows about, required first.
pub fn all_controls() -> Vec<&'static str> {
    STRICT_REQUIRED_CONTROLS
        .iter()
        .chain(STRICT_OPTIONAL_CONTROLS.iter())
        .copied()
        .collect()
}

/// Whether a control name is part of the closed control table.
pub fn is_known_control(control: &str) -> bool {
    STRICT_REQUIRED_CONTROLS.contains(&control) || STRICT_OPTIONAL_CONTROLS.contains(&control)
}
