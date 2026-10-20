//! The Linux jail: namespaces, mounts, the chroot and the PID transition.
//!
//! This is the exact port of `kala_agent.tools.sandbox.isolate_process` and
//! its helpers. The refusal messages, the error precedence and the order of
//! the operations are unchanged.
//!
//! # Call context
//!
//! `isolate_process` changes the calling process. It must run in a freshly
//! forked, single-threaded child or in the single-threaded probe helper.
//!
//! The production worker path runs the same three steps — the namespaces, the
//! PID transition and the mounts — through the native init helper of
//! [`crate::worker`], because the accepted `kala-process` launcher has no
//! PID-1 transition of its own. The steps themselves are shared, so the jail
//! a worker runs in and the jail the probe verifies are the same jail.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::constants::{MS_BIND, MS_NODEV, MS_NOEXEC, MS_NOSUID, MS_RDONLY, MS_REC, MS_REMOUNT};
use crate::error::SandboxSetupError;
use crate::path::posix_text;
use crate::probe::scratch_is_writable;
use crate::spec::SandboxSpec;

/// The strict profile requires Linux namespaces.
#[cfg(not(target_os = "linux"))]
const NON_LINUX: &str = "the strict profile requires Linux namespaces";

/// The recorded operating-system calls of the jail.
///
/// The recorder is a diagnostic: it is switched on explicitly and it never
/// changes the behaviour of a call. It exists so the differential test can
/// compare the exact jail plan — the same calls, in the same order, with the
/// same flags — against the Python implementation.
static RECORDED_CALLS: Mutex<Vec<(String, Vec<Value>)>> = Mutex::new(Vec::new());
static RECORDING: AtomicBool = AtomicBool::new(false);

/// Start recording the operating-system calls of the jail.
pub fn start_recording() {
    if let Ok(mut calls) = RECORDED_CALLS.lock() {
        calls.clear();
    }
    RECORDING.store(true, Ordering::SeqCst);
}

/// Stop recording and return the recorded calls.
pub fn take_recorded_calls() -> Vec<(String, Vec<Value>)> {
    RECORDING.store(false, Ordering::SeqCst);
    match RECORDED_CALLS.lock() {
        Ok(mut calls) => std::mem::take(&mut *calls),
        Err(_) => Vec::new(),
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn record(operation: &str, arguments: Vec<Value>) {
    if !RECORDING.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(mut calls) = RECORDED_CALLS.lock() {
        calls.push((operation.to_owned(), arguments));
    }
}

/// Build the error text of a failed operating-system call.
///
/// The text is the CPython one: `<operation> failed with errno <code>
/// (<reason>)`.
pub fn libc_error(operation: &str) -> SandboxSetupError {
    let error = std::io::Error::last_os_error();
    let code = error.raw_os_error().unwrap_or(0);
    SandboxSetupError::new(format!(
        "{operation} failed with errno {code} ({})",
        os_reason(&error)
    ))
}

/// The operating-system reason of an error, without the Rust suffix.
pub fn os_reason(error: &std::io::Error) -> String {
    let text = error.to_string();
    match text.find(" (os error") {
        Some(index) => text[..index].to_owned(),
        None => text,
    }
}

/// Create a namespace.
#[cfg(target_os = "linux")]
pub fn unshare(flag: i32, name: &str) -> Result<(), SandboxSetupError> {
    // SAFETY: `unshare` has no memory effects on this process beyond the
    // namespace change the caller asked for.
    let result = unsafe { libc::unshare(flag) };
    record("unshare", vec![Value::from(flag)]);
    if result != 0 {
        return Err(libc_error(&format!("unshare({name})")));
    }
    Ok(())
}

/// Create a namespace.
#[cfg(not(target_os = "linux"))]
pub fn unshare(_flag: i32, name: &str) -> Result<(), SandboxSetupError> {
    Err(SandboxSetupError::new(format!(
        "unshare({name}) failed with errno 95 (Operation not supported)"
    )))
}

/// Mount a filesystem.
///
/// A `None` source or filesystem becomes an empty C string, exactly like the
/// Python call, while a `None` data pointer stays null.
#[cfg(target_os = "linux")]
pub fn mount(
    source: Option<&str>,
    target: &str,
    filesystem: Option<&str>,
    flags: u64,
    data: Option<&str>,
) -> Result<(), SandboxSetupError> {
    let source_text = source.unwrap_or("").to_owned();
    let filesystem_text = filesystem.unwrap_or("").to_owned();
    let data_text = data.map(str::to_owned);
    let source = c_string(source.unwrap_or(""), "the mount source")?;
    let target_text = target.to_owned();
    let target = c_string(target, "the mount target")?;
    let filesystem = c_string(filesystem.unwrap_or(""), "the mount filesystem")?;
    let data = match data {
        Some(data) => Some(c_string(data, "the mount data")?),
        None => None,
    };
    // SAFETY: every pointer refers to a live NUL-terminated C string for the
    // duration of the call, and the null data pointer is the documented way
    // to pass no mount options.
    let result = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            filesystem.as_ptr(),
            flags as libc::c_ulong,
            data.as_ref()
                .map_or(std::ptr::null(), |value| value.as_ptr())
                .cast(),
        )
    };
    record(
        "mount",
        vec![
            Value::from(source_text.clone()),
            Value::from(target_text.clone()),
            Value::from(filesystem_text.clone()),
            Value::from(flags),
            match &data_text {
                Some(data) => Value::from(data.clone()),
                None => Value::Null,
            },
        ],
    );
    if result != 0 {
        return Err(libc_error(&format!("mount({target_text})")));
    }
    Ok(())
}

/// Mount a filesystem.
#[cfg(not(target_os = "linux"))]
pub fn mount(
    _source: Option<&str>,
    target: &str,
    _filesystem: Option<&str>,
    _flags: u64,
    _data: Option<&str>,
) -> Result<(), SandboxSetupError> {
    Err(SandboxSetupError::new(format!(
        "mount({target}) failed with errno 95 (Operation not supported)"
    )))
}

/// Create the mount point of a binding inside the jail root.
pub fn make_mountpoint(
    root: &Path,
    target: &str,
    directory: bool,
) -> Result<PathBuf, SandboxSetupError> {
    let relative = target.trim_start_matches('/');
    let mountpoint = if relative.is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    let outcome = if directory {
        make_directory(&mountpoint)
    } else {
        make_file(&mountpoint)
    };
    outcome.map(|()| mountpoint.clone()).map_err(|error| {
        SandboxSetupError::new(format!(
            "the sandbox root cannot hold {target}: {} ({})",
            os_reason(&error),
            mountpoint.display()
        ))
    })
}

fn make_directory(mountpoint: &Path) -> std::io::Result<()> {
    if mountpoint.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(mountpoint)?;
    set_mode(mountpoint, 0o700)
}

fn make_file(mountpoint: &Path) -> std::io::Result<()> {
    if let Some(parent) = mountpoint.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !mountpoint.exists() {
        std::fs::File::create(mountpoint)?;
        set_mode(mountpoint, 0o600)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// Bind a host path into the jail read-only.
pub fn bind_readonly(root: &Path, target: &str) -> Result<(), SandboxSetupError> {
    let source = Path::new(target);
    if !source.exists() {
        return Err(SandboxSetupError::new(format!(
            "the read-only binding {target} does not exist"
        )));
    }
    let mountpoint = make_mountpoint(root, target, source.is_dir())?;
    let text = posix_text(&mountpoint);
    mount(Some(target), &text, None, MS_BIND | MS_REC, None)?;
    mount(
        None,
        &text,
        None,
        MS_BIND | MS_REMOUNT | MS_RDONLY | MS_NOSUID | MS_NODEV,
        None,
    )
}

/// Bind a host directory into the jail read-write.
pub fn bind_read_write(root: &Path, target: &str) -> Result<(), SandboxSetupError> {
    let source = Path::new(target);
    if !source.is_dir() {
        return Err(SandboxSetupError::new(format!(
            "the writable binding {target} is not a directory"
        )));
    }
    let mountpoint = make_mountpoint(root, target, true)?;
    let text = posix_text(&mountpoint);
    mount(Some(target), &text, None, MS_BIND | MS_REC, None)?;
    mount(
        None,
        &text,
        None,
        MS_BIND | MS_REMOUNT | MS_NOSUID | MS_NODEV,
        None,
    )
}

/// Bind a device node into the jail.
pub fn bind_device(root: &Path, target: &str) -> Result<(), SandboxSetupError> {
    let mountpoint = make_mountpoint(root, target, false)?;
    let text = posix_text(&mountpoint);
    mount(Some(target), &text, None, MS_BIND, None)?;
    mount(
        None,
        &text,
        None,
        MS_BIND | MS_REMOUNT | MS_NOSUID | MS_NODEV,
        None,
    )
}

/// Drop the scratch paths that would hide a required binding from the child.
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

/// Mount a private tmpfs on a scratch path.
pub fn mount_scratch(
    root: &Path,
    target: &str,
    scratch_bytes: i64,
) -> Result<(), SandboxSetupError> {
    let mountpoint = make_mountpoint(root, target, true)?;
    let text = posix_text(&mountpoint);
    let size = format!("size={}", scratch_bytes.max(0));
    mount(
        Some("tmpfs"),
        &text,
        Some("tmpfs"),
        MS_NOSUID | MS_NODEV | MS_NOEXEC,
        Some(&size),
    )?;
    mount(
        None,
        &text,
        Some("tmpfs"),
        MS_REMOUNT | MS_NOSUID | MS_NODEV | MS_NOEXEC,
        Some(&size),
    )
}

/// Mount the scratch set, unmounting it again when it is not usable.
pub fn mount_scratch_set(
    root: &Path,
    targets: &[String],
    scratch_bytes: i64,
) -> Result<bool, SandboxSetupError> {
    let mut mounted: Vec<String> = Vec::new();
    for target in targets {
        if mount_scratch(root, target, scratch_bytes).is_ok() {
            mounted.push(target.clone());
        }
    }
    if mounted.is_empty() {
        return Ok(false);
    }
    let candidates: Vec<&str> = mounted.iter().map(String::as_str).collect();
    if scratch_is_writable(&candidates) {
        return Ok(true);
    }
    for target in &mounted {
        let relative = target.trim_start_matches('/');
        let path = if relative.is_empty() {
            root.to_path_buf()
        } else {
            root.join(relative)
        };
        unmount(&posix_text(&path));
    }
    Ok(false)
}

/// Unmount a path, ignoring the result exactly like the Python helper.
#[cfg(target_os = "linux")]
pub fn unmount(target: &str) {
    let Ok(text) = c_string(target, "the unmount target") else {
        return;
    };
    record("umount2", vec![Value::from(target.to_owned())]);
    // SAFETY: the pointer refers to a live NUL-terminated C string.
    unsafe {
        libc::umount2(text.as_ptr(), 0);
    }
}

/// Unmount a path, ignoring the result exactly like the Python helper.
#[cfg(not(target_os = "linux"))]
pub fn unmount(_target: &str) {}

/// The exit code of a wait status, exactly like the Python helper.
///
/// A signalled status reports `128 + signal`, a normal exit reports the exit
/// code, and a stopped status reports `1`.
pub fn exit_code_from_status(status: i32) -> i32 {
    let low = status & 0x7f;
    if low == 0 {
        return (status >> 8) & 0xff;
    }
    if low == 0x7f {
        return 1;
    }
    128 + low
}

/// Drop every descriptor above the standard streams in the forked child.
pub fn close_inherited_fds() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `close_range` only closes descriptors. It is used because
        // it is async-signal-safe and needs no allocation.
        let result = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                3 as libc::c_uint,
                4_096 as libc::c_uint,
                0 as libc::c_uint,
            )
        };
        if result == 0 {
            return;
        }
        close_inherited_fds_by_listing();
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        close_inherited_fds_by_listing();
    }
    #[cfg(not(unix))]
    {
        // The Windows launcher passes an explicit handle-inheritance list, so
        // there is no descriptor to drop here.
    }
}

#[cfg(unix)]
fn close_inherited_fds_by_listing() {
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(descriptor) = name.parse::<i32>() else {
            continue;
        };
        if descriptor > 2 {
            // SAFETY: closing an invalid descriptor only reports an error,
            // which is ignored exactly like the Python helper does.
            unsafe {
                libc::close(descriptor);
            }
        }
    }
}

/// The role of the caller after the PID-namespace transition.
///
/// `CLONE_NEWPID` only takes effect for children, so the transition needs a
/// fork: the child becomes PID 1 of the new namespace and the parent has to
/// supervise it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PidRole {
    /// The caller is PID 1 of the new PID namespace.
    Init,
    /// The caller is the parent of PID 1 and has to supervise it. The value is
    /// the host process id of PID 1.
    Supervisor(i32),
}

/// Re-parent the worker so the kernel reaps its whole process namespace.
///
/// `CLONE_NEWPID` only takes effect for children, so the pre-exec child forks
/// once more and becomes PID 1 inside the new namespace. The intermediate
/// process waits for that PID 1, releases the descriptors the supervisor uses
/// to track the direct child, and mirrors the exit status so the supervisor
/// still reaps a single process.
#[cfg(target_os = "linux")]
pub fn enter_pid_namespace() -> Result<(), SandboxSetupError> {
    match enter_pid_namespace_with_role()? {
        PidRole::Init => Ok(()),
        PidRole::Supervisor(child) => {
            close_inherited_fds();
            let code = wait_for_child_code(child);
            // SAFETY: `_exit` terminates the intermediate process without
            // running any destructor, exactly like the Python `os._exit`.
            unsafe {
                libc::_exit(std::cmp::min(255, code));
            }
        }
    }
}

/// Enter the PID namespace.
///
/// The child of the fork becomes PID 1 of the new namespace; the caller
/// becomes the parent of that PID 1 and has to supervise it.
#[cfg(target_os = "linux")]
pub fn enter_pid_namespace_with_role() -> Result<PidRole, SandboxSetupError> {
    unshare(crate::constants::CLONE_NEWPID, "pid_namespace")?;
    // SAFETY: the caller runs in a single-threaded child, so the fork copies a
    // process with exactly one thread and no lock can be held.
    let child = unsafe { libc::fork() };
    if child < 0 {
        return Err(libc_error("fork"));
    }
    if child == 0 {
        // The child is PID 1 of the new namespace. It must not outlive the
        // process that started it, and the kernel tears the namespace down
        // when PID 1 dies.
        crate::worker::arm_parent_death_signal();
        return Ok(PidRole::Init);
    }
    Ok(PidRole::Supervisor(child))
}

/// Enter the PID namespace.
#[cfg(not(target_os = "linux"))]
pub fn enter_pid_namespace_with_role() -> Result<PidRole, SandboxSetupError> {
    Err(SandboxSetupError::new(NON_LINUX))
}

/// Enter the PID namespace.
#[cfg(not(target_os = "linux"))]
pub fn enter_pid_namespace() -> Result<(), SandboxSetupError> {
    Err(SandboxSetupError::new(NON_LINUX))
}

/// Wait for a child and return the Python exit code of its status.
///
/// The wait status is the one the Python `os.waitpid` returns: a signalled
/// child reports `128 + signal`, a normal exit reports the exit code and a
/// stopped child reports `1`.
#[cfg(target_os = "linux")]
fn wait_for_child_code(child: i32) -> i32 {
    let mut status: libc::c_int = 0;
    // SAFETY: the child was created by this process and is not waited for
    // anywhere else.
    let waited = unsafe { libc::waitpid(child, &mut status, 0) };
    if waited < 0 {
        1
    } else {
        exit_code_from_status(status)
    }
}

/// Establish the requested controls in the calling process.
///
/// The caller must fail closed: a [`SandboxSetupError`] means the jail is not
/// in place and the worker must not run.
#[cfg(target_os = "linux")]
pub fn isolate_process(spec: &SandboxSpec) -> Result<Vec<String>, SandboxSetupError> {
    let mut established = unshare_namespaces(spec)?;
    if spec.controls.contains("pid_namespace") {
        match enter_pid_namespace_with_role() {
            Ok(PidRole::Init) => established.push("pid_namespace".to_owned()),
            Ok(PidRole::Supervisor(child)) => {
                // The intermediate process mirrors the exit status of PID 1
                // and never returns.
                close_inherited_fds();
                let code = wait_for_child_code(child);
                // SAFETY: `_exit` terminates the intermediate process without
                // running any destructor, exactly like the Python `os._exit`.
                unsafe {
                    libc::_exit(std::cmp::min(255, code));
                }
            }
            // The Python implementation swallows a failure of the PID
            // transition and continues without the control.
            Err(_) => {}
        }
    }
    established.extend(build_jail(spec)?);
    Ok(established)
}

/// Unshare the namespaces the specification requests, in the Python order.
///
/// The order is part of the contract: the user namespace must come first so
/// the remaining namespaces can be created with the mapped identity.
pub fn unshare_namespaces(spec: &SandboxSpec) -> Result<Vec<String>, SandboxSetupError> {
    let mut established: Vec<String> = Vec::new();
    for name in crate::constants::UNSHARE_ORDER {
        if !spec.controls.contains(*name) {
            continue;
        }
        let flag =
            crate::constants::namespace_flag(name).expect("every unshared control has a flag");
        unshare(flag, name)?;
        established.push((*name).to_owned());
    }
    Ok(established)
}

/// Build the jail of the calling process: the mounts, the chroot and the
/// working directory.
///
/// The `/proc` of the jail is mounted here, after the PID transition, so it
/// shows the processes of the new PID namespace and never the host ones.
pub fn build_jail(spec: &SandboxSpec) -> Result<Vec<String>, SandboxSetupError> {
    let mut established: Vec<String> = Vec::new();
    if spec.controls.contains("mount_namespace") {
        mount(None, "/", None, MS_REC | crate::constants::MS_PRIVATE, None)?;
    }
    if spec.controls.contains("device_nodes_minimal") {
        let mut failed = false;
        for target in &spec.device_bindings {
            if bind_device(&spec.root, target).is_err() {
                failed = true;
                break;
            }
        }
        if !failed {
            established.push("device_nodes_minimal".to_owned());
        }
    }
    for target in &spec.readonly_bindings {
        bind_readonly(&spec.root, target)?;
    }
    for target in &spec.writable_bindings {
        bind_read_write(&spec.root, target)?;
    }
    if spec.controls.contains("scratch_tmpfs") {
        let targets = unshadowed_scratch_paths(spec);
        if mount_scratch_set(&spec.root, &targets, spec.scratch_bytes)? {
            established.push("scratch_tmpfs".to_owned());
        }
    }
    if spec.controls.contains("procfs") {
        if let Ok(mountpoint) = make_mountpoint(&spec.root, "/proc", true) {
            let text = posix_text(&mountpoint);
            if mount(
                Some("proc"),
                &text,
                Some("proc"),
                MS_NOSUID | MS_NODEV | MS_NOEXEC,
                None,
            )
            .is_ok()
            {
                established.push("procfs".to_owned());
            }
        }
    }
    chroot(&posix_text(&spec.root))?;
    chdir("/", "chdir to the sandbox root")?;
    chdir(&posix_text(&spec.workspace), "chdir into the workspace")?;
    Ok(established)
}

/// Establish the requested controls in the calling process.
#[cfg(not(target_os = "linux"))]
pub fn isolate_process(_spec: &SandboxSpec) -> Result<Vec<String>, SandboxSetupError> {
    Err(SandboxSetupError::new(NON_LINUX))
}

/// Build the jail in a freshly forked child before the worker is executed.
///
/// A missing specification is not an error: the portable profile runs the
/// worker without a jail.
pub fn child_pre_exec(spec: Option<&SandboxSpec>) -> Result<(), SandboxSetupError> {
    let Some(spec) = spec else {
        return Ok(());
    };
    isolate_process(spec).map(|_established| ())
}

/// The pre-exec hook: build the jail, or terminate the child.
///
/// The diagnostic on standard error and the exit code are the Python ones, so
/// the supervisor sees the same refusal it saw before the migration.
pub fn child_pre_exec_or_exit(spec: Option<&SandboxSpec>) {
    if child_pre_exec(spec).is_err() {
        use std::io::Write;
        let _ = std::io::stderr().write_all(b"sandbox_setup_failed: SandboxSetupError\n");
        let _ = std::io::stderr().flush();
        std::process::exit(crate::constants::ISOLATION_FAILURE_EXIT);
    }
}

/// Change the root directory of the calling process.
#[cfg(target_os = "linux")]
pub fn chroot(target: &str) -> Result<(), SandboxSetupError> {
    let text = c_string(target, "the chroot target")?;
    // SAFETY: the pointer refers to a live NUL-terminated C string.
    let result = unsafe { libc::chroot(text.as_ptr()) };
    record("chroot", vec![Value::from(target.to_owned())]);
    if result != 0 {
        return Err(libc_error("chroot"));
    }
    Ok(())
}

/// Change the root directory of the calling process.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub fn chroot(_target: &str) -> Result<(), SandboxSetupError> {
    Err(SandboxSetupError::new(NON_LINUX))
}

/// Change the working directory of the calling process.
#[cfg(target_os = "linux")]
pub fn chdir(target: &str, operation: &str) -> Result<(), SandboxSetupError> {
    let text = c_string(target, "the chdir target")?;
    // SAFETY: the pointer refers to a live NUL-terminated C string.
    let result = unsafe { libc::chdir(text.as_ptr()) };
    record("chdir", vec![Value::from(target.to_owned())]);
    if result != 0 {
        return Err(libc_error(operation));
    }
    Ok(())
}

/// Change the working directory of the calling process.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub fn chdir(_target: &str, _operation: &str) -> Result<(), SandboxSetupError> {
    Err(SandboxSetupError::new(NON_LINUX))
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn c_string(value: &str, label: &str) -> Result<CString, SandboxSetupError> {
    CString::new(value)
        .map_err(|_| SandboxSetupError::new(format!("the {label} contains a null byte")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_python() {
        assert_eq!(exit_code_from_status(0), 0);
        assert_eq!(exit_code_from_status(9 << 8), 9);
        assert_eq!(exit_code_from_status(3 << 8), 3);
        assert_eq!(exit_code_from_status(0xffff), 1);
        assert_eq!(exit_code_from_status(9), 137);
    }
}
