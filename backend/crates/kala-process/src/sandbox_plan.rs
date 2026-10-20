//! Linux sandbox plan built in the parent and applied after `fork`.
//!
//! The plan is fully materialised before the fork: mount points, C strings and
//! map-file contents are prepared with ordinary safe Rust. The post-fork child
//! then performs only raw, async-signal-safe system calls — `unshare`,
//! `open`/`write`/`close`, `mount`, `chroot`, `chdir` and `execve`. No
//! allocation, formatting, locking or `std::env` access happens in that child.
//!
//! The same plan is used in two modes:
//!
//! * [`PreparedSandbox::apply_raw`] runs in the spawned worker child, where an
//!   error is written to the parent's bootstrap pipe and the child exits with
//!   [`crate::ISOLATION_FAILURE_EXIT`] before `execve`.
//! * [`PreparedSandbox::apply_safe`] runs in the separately executed,
//!   single-threaded probe helper. It additionally verifies that every
//!   requested namespace inode actually changed, so the probe cannot advertise
//!   a control merely because `unshare` returned success.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::error::SandboxSetupError;
use crate::sandbox::{
    CLONE_NEWIPC, CLONE_NEWNET, CLONE_NEWNS, CLONE_NEWUSER, CLONE_NEWUTS, MS_BIND, MS_NODEV,
    MS_NOEXEC, MS_NOSUID, MS_PRIVATE, MS_RDONLY, MS_REC, MS_REMOUNT, SandboxSpec,
    unshadowed_scratch_paths,
};

#[derive(Debug)]
pub(crate) struct SetupStep {
    pub(crate) kind: StepKind,
    pub(crate) control: Option<&'static str>,
    pub(crate) description: String,
    pub(crate) optional: bool,
}

#[derive(Debug)]
pub(crate) enum StepKind {
    Unshare {
        flag: i32,
        namespace: Option<&'static str>,
    },
    WriteFile {
        path: CString,
        data: CString,
    },
    Mount {
        source: Option<CString>,
        target: CString,
        filesystem: Option<CString>,
        flags: u64,
        data: Option<CString>,
    },
    Chroot {
        target: CString,
    },
    Chdir {
        target: CString,
    },
}

/// A complete sandbox plan.
#[derive(Debug)]
pub(crate) struct PreparedSandbox {
    steps: Vec<SetupStep>,
    namespaces: Vec<(&'static str, String)>,
}

impl PreparedSandbox {
    /// Prepare every mount point and C string before the fork. Optional
    /// controls are required to succeed in production: the executor builds the
    /// specification from controls the probe verified, so a later failure is a
    /// real containment failure rather than a silent downgrade.
    pub(crate) fn new(spec: &SandboxSpec) -> Result<Self, SandboxSetupError> {
        Self::build(spec, false)
    }

    /// Prepare a probe plan. Device, scratch and procfs mounts are optional
    /// because the Python probe treats those controls as optional and reports
    /// them as unenforced rather than failing the whole strict profile.
    pub(crate) fn new_probe(spec: &SandboxSpec) -> Result<Self, SandboxSetupError> {
        Self::build(spec, true)
    }

    fn build(spec: &SandboxSpec, optional_controls: bool) -> Result<Self, SandboxSetupError> {
        spec.validate_strict()?;
        let root = &spec.root;
        let workspace = std::fs::canonicalize(&spec.workspace).map_err(|_| {
            SandboxSetupError::new("the sandbox workspace is not a usable directory")
        })?;
        if !workspace.is_dir() {
            return Err(SandboxSetupError::new(
                "the sandbox workspace is not a usable directory",
            ));
        }
        if !root.is_absolute() {
            return Err(SandboxSetupError::new("the sandbox root must be absolute"));
        }
        std::fs::create_dir_all(root).map_err(|error| {
            SandboxSetupError::new(format!("the sandbox root cannot be created: {error}"))
        })?;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| SandboxSetupError::new(format!("the sandbox root cannot be secured: {error}")),
        )?;
        let mut steps = Vec::new();
        let mut namespaces = Vec::new();
        for (control, flag) in [
            ("user_namespace", CLONE_NEWUSER),
            ("network_isolation", CLONE_NEWNET),
            ("mount_namespace", CLONE_NEWNS),
            ("ipc_namespace", CLONE_NEWIPC),
            ("uts_namespace", CLONE_NEWUTS),
        ] {
            if !spec.controls.contains(control) {
                continue;
            }
            let namespace = namespace_name(control);
            steps.push(SetupStep {
                kind: StepKind::Unshare { flag, namespace },
                control: Some(control),
                description: format!("unshare({control})"),
                optional: false,
            });
            if let Some(namespace) = namespace {
                namespaces.push((control, namespace.to_owned()));
            }
            if control == "user_namespace" {
                add_user_maps(&mut steps)?;
            }
        }
        if spec.controls.contains("mount_namespace") {
            steps.push(SetupStep {
                kind: StepKind::Mount {
                    source: None,
                    target: cstring("/", "mount private root")?,
                    filesystem: None,
                    flags: MS_REC | MS_PRIVATE,
                    data: None,
                },
                control: None,
                description: "mount private /".to_owned(),
                optional: false,
            });
        }
        if spec.controls.contains("device_nodes_minimal") {
            for target in &spec.device_bindings {
                if !Path::new(target).exists() {
                    return Err(SandboxSetupError::new(format!(
                        "the device binding {target} does not exist"
                    )));
                }
                let mountpoint = make_mountpoint(root, target, MountpointKind::File)?;
                add_bind(
                    &mut steps,
                    target,
                    &mountpoint,
                    "device_nodes_minimal",
                    "device",
                    false,
                    optional_controls,
                )?;
            }
        }
        for target in &spec.readonly_bindings {
            if !Path::new(target).exists() {
                return Err(SandboxSetupError::new(format!(
                    "the read-only binding {target} does not exist"
                )));
            }
            let kind = if Path::new(target).is_dir() {
                MountpointKind::Directory
            } else {
                MountpointKind::File
            };
            let mountpoint = make_mountpoint(root, target, kind)?;
            add_bind(
                &mut steps,
                target,
                &mountpoint,
                "readonly_system_bindings",
                "read-only",
                true,
                false,
            )?;
        }
        for target in &spec.writable_bindings {
            if !Path::new(target).is_dir() {
                return Err(SandboxSetupError::new(format!(
                    "the writable binding {target} is not a directory"
                )));
            }
            let mountpoint = make_mountpoint(root, target, MountpointKind::Directory)?;
            add_bind(
                &mut steps,
                target,
                &mountpoint,
                "workspace_read_write",
                "writable",
                false,
                false,
            )?;
        }
        if spec.controls.contains("scratch_tmpfs") {
            for target in unshadowed_scratch_paths(spec) {
                let mountpoint = make_mountpoint(root, &target, MountpointKind::Directory)?;
                let size = format!("size={}", spec.scratch_bytes.max(0));
                let source = cstring("tmpfs", "scratch source")?;
                let filesystem = cstring("tmpfs", "scratch filesystem")?;
                let data = cstring(&size, "scratch size")?;
                steps.push(SetupStep {
                    kind: StepKind::Mount {
                        source: Some(source.clone()),
                        target: cstring(&path_text(&mountpoint), "scratch mountpoint")?,
                        filesystem: Some(filesystem.clone()),
                        flags: MS_NOSUID | MS_NODEV | MS_NOEXEC,
                        data: Some(data.clone()),
                    },
                    control: Some("scratch_tmpfs"),
                    description: format!("mount scratch tmpfs at {target}"),
                    optional: optional_controls,
                });
                steps.push(SetupStep {
                    kind: StepKind::Mount {
                        source: Some(source),
                        target: cstring(&path_text(&mountpoint), "scratch remount")?,
                        filesystem: Some(filesystem),
                        flags: MS_REMOUNT | MS_NOSUID | MS_NODEV | MS_NOEXEC,
                        data: Some(data),
                    },
                    control: Some("scratch_tmpfs"),
                    description: format!("remount scratch tmpfs at {target}"),
                    optional: optional_controls,
                });
            }
        }
        if spec.controls.contains("procfs") {
            let mountpoint = make_mountpoint(root, "/proc", MountpointKind::Directory)?;
            steps.push(SetupStep {
                kind: StepKind::Mount {
                    source: Some(cstring("proc", "procfs source")?),
                    target: cstring(&path_text(&mountpoint), "procfs mountpoint")?,
                    filesystem: Some(cstring("proc", "procfs filesystem")?),
                    flags: MS_NOSUID | MS_NODEV | MS_NOEXEC,
                    data: None,
                },
                control: Some("procfs"),
                description: "mount procfs".to_owned(),
                optional: optional_controls,
            });
        }
        steps.push(SetupStep {
            kind: StepKind::Chroot {
                target: cstring(&path_text(root), "chroot target")?,
            },
            control: Some("chroot_workspace_only"),
            description: "chroot into the sandbox root".to_owned(),
            optional: false,
        });
        steps.push(SetupStep {
            kind: StepKind::Chdir {
                target: cstring("/", "chroot root")?,
            },
            control: None,
            description: "chdir to the sandbox root".to_owned(),
            optional: false,
        });
        steps.push(SetupStep {
            kind: StepKind::Chdir {
                target: cstring(&path_text(&workspace), "workspace chdir")?,
            },
            control: None,
            description: "chdir into the workspace".to_owned(),
            optional: false,
        });
        Ok(Self { steps, namespaces })
    }

    pub(crate) fn step_description(&self, index: usize) -> &str {
        self.steps
            .get(index)
            .map(|step| step.description.as_str())
            .unwrap_or("unknown sandbox step")
    }

    /// Apply the plan in the single-threaded probe helper and report the
    /// controls actually established.
    pub(crate) fn apply_safe(&self) -> Result<Vec<String>, SandboxSetupError> {
        let baseline = capture_namespaces(&self.namespaces)?;
        let mut established: Vec<String> = Vec::new();
        let mut failed_controls: BTreeSet<&'static str> = BTreeSet::new();
        for step in &self.steps {
            match &step.kind {
                StepKind::Unshare { flag, namespace } => {
                    if unsafe { libc::unshare(*flag) } != 0 {
                        if step.optional {
                            if let Some(control) = step.control {
                                failed_controls.insert(control);
                            }
                            continue;
                        }
                        return Err(errno_error(&step.description));
                    }
                    if let Some(namespace) = namespace {
                        let before = baseline.get(*namespace).copied().ok_or_else(|| {
                            SandboxSetupError::new("namespace baseline is missing")
                        })?;
                        let after = namespace_inode(namespace).map_err(|error| {
                            SandboxSetupError::new(format!(
                                "the {} namespace could not be verified: {error}",
                                step.description
                            ))
                        })?;
                        if before == after {
                            if step.optional {
                                if let Some(control) = step.control {
                                    failed_controls.insert(control);
                                }
                                continue;
                            }
                            return Err(SandboxSetupError::new(format!(
                                "{} did not change namespace identity",
                                step.description
                            )));
                        }
                    }
                }
                StepKind::WriteFile { path, data } => {
                    let path = path.to_str().map_err(|_| {
                        SandboxSetupError::new("the sandbox map path is not valid UTF-8")
                    })?;
                    let data = data.to_str().map_err(|_| {
                        SandboxSetupError::new("the sandbox map payload is not valid UTF-8")
                    })?;
                    if let Err(error) = std::fs::write(path, data) {
                        if step.optional {
                            if let Some(control) = step.control {
                                failed_controls.insert(control);
                            }
                            continue;
                        }
                        return Err(SandboxSetupError::new(format!(
                            "{} failed: {error}",
                            step.description
                        )));
                    }
                }
                StepKind::Mount {
                    source,
                    target,
                    filesystem,
                    flags,
                    data,
                } => {
                    let mounted = unsafe {
                        libc::mount(
                            source
                                .as_ref()
                                .map_or(std::ptr::null(), |value| value.as_ptr()),
                            target.as_ptr(),
                            filesystem
                                .as_ref()
                                .map_or(std::ptr::null(), |value| value.as_ptr()),
                            *flags as libc::c_ulong,
                            data.as_ref()
                                .map_or(std::ptr::null(), |value| value.as_ptr())
                                .cast(),
                        )
                    };
                    if mounted != 0 {
                        if step.optional {
                            if let Some(control) = step.control {
                                failed_controls.insert(control);
                            }
                            continue;
                        }
                        return Err(errno_error(&step.description));
                    }
                }
                StepKind::Chroot { target } => {
                    if unsafe { libc::chroot(target.as_ptr()) } != 0 {
                        if step.optional {
                            if let Some(control) = step.control {
                                failed_controls.insert(control);
                            }
                            continue;
                        }
                        return Err(errno_error(&step.description));
                    }
                }
                StepKind::Chdir { target } => {
                    if unsafe { libc::chdir(target.as_ptr()) } != 0 {
                        if step.optional {
                            if let Some(control) = step.control {
                                failed_controls.insert(control);
                            }
                            continue;
                        }
                        return Err(errno_error(&step.description));
                    }
                }
            }
            if let Some(control) = step.control {
                if !failed_controls.contains(control)
                    && !established.iter().any(|value| value == control)
                {
                    established.push(control.to_owned());
                }
            }
        }
        established.retain(|control| !failed_controls.contains(control.as_str()));
        Ok(established)
    }

    /// Apply the plan in the post-fork child.
    ///
    /// # Safety
    ///
    /// The caller must be the single forked child of a Unix `fork`, before
    /// `execve`, and must not have created threads in that child. Every
    /// operation is a raw system call or a C-string read; no allocation,
    /// formatting or locking occurs.
    pub(crate) unsafe fn apply_raw(&self) -> Result<(), (usize, i32)> {
        for (index, step) in self.steps.iter().enumerate() {
            let result = match &step.kind {
                StepKind::Unshare { flag, .. } => unsafe { libc::unshare(*flag) },
                StepKind::WriteFile { path, data } => unsafe {
                    write_map_file(path.as_ptr(), data.as_bytes())
                },
                StepKind::Mount {
                    source,
                    target,
                    filesystem,
                    flags,
                    data,
                } => unsafe {
                    libc::mount(
                        source
                            .as_ref()
                            .map_or(std::ptr::null(), |value| value.as_ptr()),
                        target.as_ptr(),
                        filesystem
                            .as_ref()
                            .map_or(std::ptr::null(), |value| value.as_ptr()),
                        *flags as libc::c_ulong,
                        data.as_ref()
                            .map_or(std::ptr::null(), |value| value.as_ptr())
                            .cast(),
                    )
                },
                StepKind::Chroot { target } => unsafe { libc::chroot(target.as_ptr()) },
                StepKind::Chdir { target } => unsafe { libc::chdir(target.as_ptr()) },
            };
            if result != 0 {
                return Err((index, unsafe { *libc::__errno_location() }));
            }
        }
        Ok(())
    }
}

fn add_user_maps(steps: &mut Vec<SetupStep>) -> Result<(), SandboxSetupError> {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    if Path::new("/proc/self/setgroups").exists() {
        steps.push(SetupStep {
            kind: StepKind::WriteFile {
                path: cstring("/proc/self/setgroups", "setgroups path")?,
                data: cstring("deny", "setgroups content")?,
            },
            control: None,
            description: "write /proc/self/setgroups".to_owned(),
            optional: false,
        });
    }
    steps.push(SetupStep {
        kind: StepKind::WriteFile {
            path: cstring("/proc/self/uid_map", "uid_map path")?,
            data: cstring(&format!("0 {uid} 1"), "uid_map content")?,
        },
        control: None,
        description: "write /proc/self/uid_map".to_owned(),
        optional: false,
    });
    steps.push(SetupStep {
        kind: StepKind::WriteFile {
            path: cstring("/proc/self/gid_map", "gid_map path")?,
            data: cstring(&format!("0 {gid} 1"), "gid_map content")?,
        },
        control: None,
        description: "write /proc/self/gid_map".to_owned(),
        optional: false,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_bind(
    steps: &mut Vec<SetupStep>,
    source: &str,
    mountpoint: &Path,
    control: &'static str,
    label: &str,
    readonly: bool,
    optional: bool,
) -> Result<(), SandboxSetupError> {
    let target = cstring(&path_text(mountpoint), "bind mountpoint")?;
    steps.push(SetupStep {
        kind: StepKind::Mount {
            source: Some(cstring(source, "bind source")?),
            target: target.clone(),
            filesystem: None,
            flags: MS_BIND | MS_REC,
            data: None,
        },
        control: Some(control),
        description: format!("{label} bind {source}"),
        optional,
    });
    let flags = if readonly {
        MS_BIND | MS_REMOUNT | MS_RDONLY | MS_NOSUID | MS_NODEV
    } else {
        MS_BIND | MS_REMOUNT | MS_NOSUID | MS_NODEV
    };
    steps.push(SetupStep {
        kind: StepKind::Mount {
            source: None,
            target,
            filesystem: None,
            flags,
            data: None,
        },
        control: Some(control),
        description: format!("{label} remount {source}"),
        optional,
    });
    Ok(())
}

#[derive(Clone, Copy)]
enum MountpointKind {
    Directory,
    File,
}

fn make_mountpoint(
    root: &Path,
    target: &str,
    kind: MountpointKind,
) -> Result<PathBuf, SandboxSetupError> {
    validate_binding_target(target)?;
    let mountpoint = root.join(target.trim_start_matches('/'));
    if !mountpoint.starts_with(root) {
        return Err(SandboxSetupError::new(format!(
            "the sandbox binding {target} escapes the sandbox root"
        )));
    }
    if let Some(parent) = mountpoint.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            SandboxSetupError::new(format!("the sandbox root cannot hold {target}: {error}"))
        })?;
    }
    match kind {
        MountpointKind::Directory => {
            std::fs::create_dir_all(&mountpoint).map_err(|error| {
                SandboxSetupError::new(format!("the sandbox root cannot hold {target}: {error}"))
            })?;
        }
        MountpointKind::File => {
            if !mountpoint.exists() {
                std::fs::File::create(&mountpoint).map_err(|error| {
                    SandboxSetupError::new(format!(
                        "the sandbox root cannot hold {target}: {error}"
                    ))
                })?;
            }
        }
    }
    Ok(mountpoint)
}

fn validate_binding_target(target: &str) -> Result<(), SandboxSetupError> {
    if !target.starts_with('/') || target.trim_end_matches('/').is_empty() || target.contains('\0')
    {
        return Err(SandboxSetupError::new(format!(
            "the sandbox binding {target} is not a safe absolute path"
        )));
    }
    if target
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(SandboxSetupError::new(format!(
            "the sandbox binding {target} contains a traversal segment"
        )));
    }
    Ok(())
}

fn cstring(value: &str, label: &str) -> Result<CString, SandboxSetupError> {
    CString::new(value)
        .map_err(|_| SandboxSetupError::new(format!("the {label} contains a null byte")))
}

fn errno_error(operation: &str) -> SandboxSetupError {
    let error = std::io::Error::last_os_error();
    let code = error.raw_os_error().unwrap_or(0);
    SandboxSetupError::new(format!("{operation} failed with errno {code} ({})", error))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn namespace_name(control: &str) -> Option<&'static str> {
    match control {
        "user_namespace" => Some("user"),
        "network_isolation" => Some("net"),
        "mount_namespace" => Some("mnt"),
        "ipc_namespace" => Some("ipc"),
        "uts_namespace" => Some("uts"),
        _ => None,
    }
}

fn capture_namespaces(
    namespaces: &[(&'static str, String)],
) -> Result<BTreeMap<String, (u64, u64)>, SandboxSetupError> {
    let mut baseline = BTreeMap::new();
    for (_control, namespace) in namespaces {
        let inode = namespace_inode(namespace).map_err(|error| {
            SandboxSetupError::new(format!(
                "the {namespace} namespace cannot be verified: {error}"
            ))
        })?;
        baseline.insert(namespace.clone(), inode);
    }
    Ok(baseline)
}

fn namespace_inode(namespace: &str) -> std::io::Result<(u64, u64)> {
    let metadata = std::fs::metadata(format!("/proc/self/ns/{namespace}"))?;
    Ok((metadata.dev(), metadata.ino()))
}

/// Apply the in-place probe plan. Kept here so the helper and the production
/// pre-exec path share one plan builder.
pub(crate) fn isolate_in_place(spec: &SandboxSpec) -> Result<Vec<String>, SandboxSetupError> {
    let prepared = PreparedSandbox::new_probe(spec)?;
    prepared.apply_safe()
}

/// Write a small map file with raw descriptors. Used only after `fork`.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated C string and the caller must be the
/// post-fork child. No allocation is performed.
unsafe fn write_map_file(path: *const libc::c_char, data: &[u8]) -> i32 {
    let descriptor = unsafe { libc::open(path, libc::O_WRONLY | libc::O_CLOEXEC) };
    if descriptor < 0 {
        return -1;
    }
    let mut offset = 0usize;
    while offset < data.len() {
        let written = unsafe {
            libc::write(
                descriptor,
                data[offset..].as_ptr().cast(),
                data.len() - offset,
            )
        };
        if written < 0 {
            let error = unsafe { *libc::__errno_location() };
            if error == libc::EINTR {
                continue;
            }
            unsafe { libc::close(descriptor) };
            unsafe { *libc::__errno_location() = error };
            return -1;
        }
        if written == 0 {
            unsafe { libc::close(descriptor) };
            return -1;
        }
        offset += written as usize;
    }
    if unsafe { libc::close(descriptor) } != 0 {
        return -1;
    }
    0
}
