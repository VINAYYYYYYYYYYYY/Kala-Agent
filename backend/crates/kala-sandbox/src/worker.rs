//! The production worker jail: the native init helper.
//!
//! The accepted `kala-process` launcher builds a jail after `fork` and before
//! `execve`, but it has no PID-1 transition: a worker it launches shares the
//! host process table, and the strict profile cannot promise that the worker
//! cannot see the host processes. The missing piece is added here, without
//! touching the accepted crate: a tiny native init helper
//! (`kala-sandbox-init`) that the accepted supervisor launches *instead of*
//! the worker.
//!
//! The helper performs the Python transition:
//!
//! 1. it unshares the namespaces of the specification, in the Python order;
//! 2. it unshares the PID namespace and forks, so that the child becomes PID 1
//!    of the new namespace;
//! 3. PID 1 builds the jail — the mounts, the `/proc` of the new namespace,
//!    the `chroot` and the working directory — exactly like the Python
//!    `isolate_process`;
//! 4. PID 1 forks the worker, forwards the termination signals to it, reaps
//!    it and every orphan it leaves behind, and ends exactly as the worker
//!    ended.
//!
//! The process the supervisor started — the parent of PID 1 — waits for PID 1
//! and ends exactly as PID 1 ended, so the supervisor observes the same exit
//! status it observed when it launched the worker directly.
//!
//! # Containment
//!
//! * The worker is a child of PID 1, so an orphan is reparented to the
//!   namespace init and reaped by it.
//! * The kernel tears the whole namespace down when PID 1 dies, so no
//!   descendant can outlive the jail.
//! * PID 1 and the worker ask the kernel to kill them when their parent dies,
//!   so the namespace dies with the process the supervisor started.
//! * Every level of the chain stays in the process group of the direct child,
//!   so the supervisor's `SIGKILL` reaches all of them.
//!
//! # Platform
//!
//! The strict profile is Linux only. On every other platform the helper
//! refuses to build a jail.

use std::ffi::OsString;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicI32, Ordering};

use kala_process::{
    CancelToken, ChildEnvironment, ProcessError, ProcessLimits, ProcessOutcome, ProcessSpec,
    supervise,
};

use crate::constants::{ISOLATION_FAILURE_EXIT, PROBE_FAILURE_EXIT};
use crate::error::SandboxSetupError;
use crate::isolation::{PidRole, build_jail, close_inherited_fds, unshare_namespaces};
use crate::parse::parse_json;
use crate::spec::SandboxSpec;

/// The exit code the helper uses when the worker could not be executed.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const EXEC_FAILURE_EXIT: i32 = 127;

/// The `prctl` option that arms the parent-death signal.
#[cfg(target_os = "linux")]
const PR_SET_PDEATHSIG: libc::c_int = 1;

/// The command a jailed worker runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerCommand {
    /// The absolute path of the worker executable.
    ///
    /// Inside the jail the path is resolved against the jail root, exactly
    /// like the launcher's `execve` after the `chroot`.
    pub executable: PathBuf,
    /// The argv of the worker, excluding the executable.
    pub arguments: Vec<OsString>,
}

impl WorkerCommand {
    /// Build a command from an executable and its arguments.
    pub fn new(executable: impl Into<PathBuf>, arguments: Vec<OsString>) -> Self {
        Self {
            executable: executable.into(),
            arguments,
        }
    }
}

/// The production worker jail: the specification, the init helper and the
/// command the worker runs.
///
/// The jail is the strict profile: the specification must request the
/// `pid_namespace` control, because a production worker that shares the host
/// process table is exactly the regression this module repairs.
#[derive(Debug, Clone)]
pub struct WorkerJail {
    /// The native init helper that builds the jail.
    ///
    /// It must be the absolute path of the `kala-sandbox-init` binary.
    pub init: PathBuf,
    /// The command the worker runs inside the jail.
    pub command: WorkerCommand,
    /// The specification of the jail.
    pub spec: SandboxSpec,
    /// The environment of the worker.
    ///
    /// `None` uses the minimal native environment of the workspace, which is
    /// the environment the accepted supervisor builds by default.
    pub environment: Option<ChildEnvironment>,
    /// The supervision limits.
    ///
    /// `None` uses the accepted defaults: the wall-clock deadline, the
    /// bootstrap deadline, the termination deadline and the output caps.
    pub limits: Option<ProcessLimits>,
}

impl WorkerJail {
    /// The process specification that runs the worker inside the jail.
    ///
    /// The specification is refused when it does not request the
    /// `pid_namespace` control: the production worker jail always has a PID
    /// namespace, and a jail without one is never handed to a worker.
    pub fn process_spec(&self) -> Result<ProcessSpec, ProcessError> {
        if !self.spec.controls.contains("pid_namespace") {
            return Err(ProcessError::SandboxSetup(
                kala_process::SandboxSetupError::new(
                    "the production worker jail requires the pid_namespace control",
                ),
            ));
        }
        let mut process = ProcessSpec::new(&self.init, &self.spec.workspace)?;
        if let Some(environment) = &self.environment {
            process = process.with_environment(environment.clone());
        }
        if let Some(limits) = &self.limits {
            process = process.with_limits(limits.clone());
        }
        Ok(process
            .arg("--spec")
            .arg(self.spec.to_json())
            .arg("--")
            .arg(self.command.executable.as_os_str())
            .args(self.command.arguments.iter()))
    }

    /// Run the worker inside the jail under the accepted supervisor.
    ///
    /// The deadline, the cancellation and the output bounds are the ones the
    /// rest of the backend already uses: the supervisor supervises the init
    /// helper, which supervises the worker.
    pub fn run(&self, stdin: &[u8], cancel: &CancelToken) -> Result<ProcessOutcome, ProcessError> {
        let process = self.process_spec()?;
        supervise(&process, stdin, cancel)
    }
}

/// What the init helper was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitRequest {
    /// Build the jail, verify the controls from the inside and report them.
    Probe {
        /// The specification payload.
        spec: String,
        /// The canary payload.
        canary: String,
    },
    /// Build the jail and run the worker inside it.
    Worker {
        /// The specification payload.
        spec: String,
        /// The command the worker runs.
        command: WorkerCommand,
    },
}

/// A refusal of the init helper: the message and the exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitRefusal {
    /// The short reason.
    pub message: String,
    /// The exit code the helper ends with.
    pub code: i32,
}

impl InitRefusal {
    /// The refusal of a malformed probe request.
    fn probe(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: PROBE_FAILURE_EXIT,
        }
    }

    /// The refusal of a malformed worker request.
    fn worker(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: ISOLATION_FAILURE_EXIT,
        }
    }
}

/// Parse the arguments of the init helper.
///
/// The helper understands two requests:
///
/// * `--probe <spec> <canary>` — build the jail, verify it and report the
///   controls, exactly like the `kala-sandbox-probe` helper;
/// * `--spec <spec> -- <worker> [arguments...]` — build the jail and run the
///   worker inside it.
///
/// A request without a flag is the probe request of the `kala-sandbox-probe`
/// helper, so the init helper is a drop-in replacement for it.
pub fn parse_init_arguments(arguments: &[OsString]) -> Result<InitRequest, InitRefusal> {
    let Some(first) = arguments.first() else {
        return Err(InitRefusal::probe("the probe payload is malformed"));
    };
    let flag = first.to_string_lossy().into_owned();
    match flag.as_str() {
        "--probe" => {
            if arguments.len() < 3 {
                return Err(InitRefusal::probe("the probe payload is malformed"));
            }
            Ok(InitRequest::Probe {
                spec: text_of(&arguments[1]),
                canary: text_of(&arguments[2]),
            })
        }
        "--spec" => {
            if arguments.len() < 4 {
                return Err(InitRefusal::worker(
                    "the init helper needs a specification and a worker",
                ));
            }
            if arguments[2] != "--" {
                return Err(InitRefusal::worker(
                    "the init helper needs `--` before the worker command",
                ));
            }
            Ok(InitRequest::Worker {
                spec: text_of(&arguments[1]),
                command: WorkerCommand::new(PathBuf::from(&arguments[3]), arguments[4..].to_vec()),
            })
        }
        _ if flag.starts_with("--") => Err(InitRefusal::worker(format!(
            "the init helper does not understand {flag}"
        ))),
        _ => {
            if arguments.len() < 2 {
                return Err(InitRefusal::probe("the probe payload is malformed"));
            }
            Ok(InitRequest::Probe {
                spec: text_of(&arguments[0]),
                canary: text_of(&arguments[1]),
            })
        }
    }
}

/// The entry point of the native init helper.
///
/// The helper is the process the accepted supervisor starts. It builds the
/// jail and then runs the worker inside it, or verifies the jail and reports
/// the controls it established.
pub fn run_init(arguments: &[OsString]) -> i32 {
    match parse_init_arguments(arguments) {
        Ok(InitRequest::Probe { spec, canary }) => crate::probe_run::run_probe(&spec, &canary),
        Ok(InitRequest::Worker { spec, command }) => run_worker(&spec, &command),
        Err(refusal) => {
            eprintln!("{}", refusal.message);
            refusal.code
        }
    }
}

/// Build the jail of the specification and run the worker inside it.
pub fn run_worker(spec_payload: &str, command: &WorkerCommand) -> i32 {
    let spec = match parse_spec(spec_payload) {
        Ok(spec) => spec,
        Err(error) => return setup_failure(&error),
    };
    run_worker_in_jail(&spec, command)
}

/// Decode the specification of the worker request.
fn parse_spec(spec_payload: &str) -> Result<SandboxSpec, SandboxSetupError> {
    let decoded = parse_json(spec_payload).map_err(|_| SandboxSetupError::malformed())?;
    SandboxSpec::from_payload(&decoded)
}

/// The controls the jail establishes itself.
///
/// The other controls of the closed table — `chroot_workspace_only`,
/// `readonly_system_bindings` and `workspace_read_write` — are established by
/// the `chroot` and the bindings, which fail closed on their own.
const SETUP_CONTROLS: &[&str] = &[
    "device_nodes_minimal",
    "ipc_namespace",
    "mount_namespace",
    "network_isolation",
    "pid_namespace",
    "procfs",
    "scratch_tmpfs",
    "user_namespace",
    "uts_namespace",
];

/// Build the jail and run the worker inside it.
///
/// The function never returns in the supervisor role: the process ends
/// exactly as the worker ended. In the init role it returns the exit code
/// the supervisor has to report.
pub fn run_worker_in_jail(spec: &SandboxSpec, command: &WorkerCommand) -> i32 {
    let mut established = match unshare_namespaces(spec) {
        Ok(established) => established,
        Err(error) => return setup_failure(&error),
    };
    let mut channel = StatusChannel::none();
    if spec.controls.contains("pid_namespace") {
        // The supervisor of the namespace watches for the death of its parent
        // with a pipe, so the descriptors above the standard streams are
        // released before the pipe is created.
        close_inherited_fds();
        let mut created = match StatusChannel::create() {
            Ok(created) => created,
            Err(error) => return setup_failure(&error),
        };
        match enter_pid_namespace() {
            Ok(PidRole::Init) => {
                // The init of the namespace: it keeps the write end of the
                // status channel.
                created.close_read();
                channel = Some(created);
                established.push("pid_namespace".to_owned());
            }
            Ok(PidRole::Supervisor(child)) => {
                // The parent of the namespace init: it keeps the read end of
                // the status channel. This never returns.
                created.close_write();
                supervise_namespace_init(child, &created);
            }
            // The production path fails closed: a worker never runs without the
            // PID namespace it was promised.
            Err(error) => return setup_failure(&error),
        }
    }
    match build_jail(spec) {
        Ok(jail) => established.extend(jail),
        Err(error) => return setup_failure(&error),
    }
    // Every control the specification asks for must be established. The
    // Python implementation continues without the controls it could not
    // establish and reports them; the production path refuses, because a
    // worker that runs without the promised `/proc` would see the host
    // process table.
    for control in SETUP_CONTROLS {
        if spec.controls.contains(*control) && !established.iter().any(|value| value == control) {
            return setup_failure(&SandboxSetupError::new(format!(
                "the production worker jail could not establish the {control} control"
            )));
        }
    }
    supervise_worker(command, channel.as_ref())
}

/// Unshare the PID namespace and fork, so that the child becomes PID 1.
///
/// The channel carries the status of the worker from the init to the parent.
#[cfg(target_os = "linux")]
fn enter_pid_namespace() -> Result<PidRole, SandboxSetupError> {
    crate::isolation::unshare(crate::constants::CLONE_NEWPID, "pid_namespace")?;
    // SAFETY: the caller runs in a single-threaded child, so the fork copies a
    // process with exactly one thread and no lock can be held.
    let child = unsafe { libc::fork() };
    if child < 0 {
        return Err(crate::isolation::libc_error("fork"));
    }
    if child == 0 {
        // The child is PID 1 of the new namespace. It must not outlive the
        // process that started it, and the kernel tears the namespace down
        // when PID 1 dies.
        arm_parent_death_signal();
        return Ok(PidRole::Init);
    }
    Ok(PidRole::Supervisor(child))
}

/// Unshare the PID namespace and fork, so that the child becomes PID 1.
#[cfg(not(target_os = "linux"))]
fn enter_pid_namespace() -> Result<PidRole, SandboxSetupError> {
    Err(SandboxSetupError::new(
        "the strict profile requires Linux namespaces",
    ))
}

/// The channel that carries the status of the worker from the namespace init
/// to the process the supervisor started.
///
/// The init of a PID namespace cannot be killed by a fatal signal that is sent
/// from inside the namespace — the kernel ignores it — so the init cannot
/// repeat the death of the worker. It writes the status of the worker into
/// this pipe instead, and the parent, which is not the init of a namespace,
/// repeats it. The supervisor therefore observes the same death it observed
/// when it launched the worker directly.
#[derive(Debug)]
struct StatusChannel {
    /// The read end, held by the parent of the init.
    read: i32,
    /// The write end, held by the init.
    write: i32,
}

impl StatusChannel {
    /// The channel of a jail without a PID namespace.
    fn none() -> Option<Self> {
        None
    }

    /// Create the channel.
    ///
    /// Both ends are close-on-exec, so the worker never keeps the channel
    /// open: the parent sees the end of the file as soon as the init is gone.
    #[cfg(target_os = "linux")]
    fn create() -> Result<Self, SandboxSetupError> {
        let mut descriptors: [i32; 2] = [0; 2];
        // SAFETY: the array is a valid output location.
        let result = unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) };
        if result != 0 {
            return Err(crate::isolation::libc_error("pipe2"));
        }
        Ok(Self {
            read: descriptors[0],
            write: descriptors[1],
        })
    }

    /// Create the channel.
    #[cfg(not(target_os = "linux"))]
    fn create() -> Result<Self, SandboxSetupError> {
        Err(SandboxSetupError::new(
            "the strict profile requires Linux namespaces",
        ))
    }

    /// Close the read end.
    #[cfg(target_os = "linux")]
    fn close_read(&mut self) {
        if self.read >= 0 {
            // SAFETY: the descriptor is owned by the channel.
            unsafe {
                libc::close(self.read);
            }
            self.read = -1;
        }
    }

    /// Close the read end.
    #[cfg(not(target_os = "linux"))]
    fn close_read(&mut self) {
        self.read = -1;
    }

    /// Close the write end.
    #[cfg(target_os = "linux")]
    fn close_write(&mut self) {
        if self.write >= 0 {
            // SAFETY: the descriptor is owned by the channel.
            unsafe {
                libc::close(self.write);
            }
            self.write = -1;
        }
    }

    /// Close the write end.
    #[cfg(not(target_os = "linux"))]
    fn close_write(&mut self) {
        self.write = -1;
    }

    /// Report the status of the worker to the parent of the init.
    ///
    /// A failure is ignored: the parent falls back to the status of the init
    /// itself.
    #[cfg(target_os = "linux")]
    fn write_status(&self, status: i32) {
        if self.write < 0 {
            return;
        }
        let bytes = status.to_le_bytes();
        // SAFETY: the descriptor is owned by the channel and the buffer is
        // valid for the call.
        let written = unsafe { libc::write(self.write, bytes.as_ptr().cast(), bytes.len()) };
        if written < 0 {
            // The parent is gone: the pipe is broken and the namespace is
            // torn down by the kernel.
        }
    }

    /// Report the status of the worker to the parent of the init.
    #[cfg(not(target_os = "linux"))]
    #[allow(dead_code)]
    fn write_status(&self, _status: i32) {}

    /// Read the status the init reported, if it reported one.
    #[cfg(target_os = "linux")]
    fn read_status(&self) -> Option<i32> {
        if self.read < 0 {
            return None;
        }
        let mut bytes = [0u8; 4];
        // SAFETY: the descriptor is owned by the channel and the buffer is
        // valid for the call.
        let read = unsafe { libc::read(self.read, bytes.as_mut_ptr().cast(), bytes.len()) };
        if read == 4 {
            Some(i32::from_le_bytes(bytes))
        } else {
            None
        }
    }

    /// Read the status the init reported, if it reported one.
    #[cfg(not(target_os = "linux"))]
    #[allow(dead_code)]
    fn read_status(&self) -> Option<i32> {
        None
    }
}

/// Report a setup failure and end the process with the isolation code.
///
/// The diagnostic on standard error is the Python one, byte for byte:
/// `sandbox_setup_failed: SandboxSetupError`. The reason itself is not
/// written, exactly like the Python pre-exec hook, so that no host path
/// reaches the captured stderr of a worker.
fn setup_failure(_error: &SandboxSetupError) -> i32 {
    use std::io::Write;
    let _ = std::io::stderr().write_all(b"sandbox_setup_failed: SandboxSetupError\n");
    let _ = std::io::stderr().flush();
    ISOLATION_FAILURE_EXIT
}

/// The process that forwards the termination signals to its child.
#[cfg(target_os = "linux")]
static FORWARD_TARGET: AtomicI32 = AtomicI32::new(0);

/// Forward a termination signal to the supervised child.
///
/// The handler only calls `kill`, which is async-signal-safe.
#[cfg(target_os = "linux")]
extern "C" fn forward_signal(signal: libc::c_int) {
    let target = FORWARD_TARGET.load(Ordering::SeqCst);
    if target > 0 {
        // SAFETY: `kill` has no memory-safety preconditions.
        unsafe {
            libc::kill(target, signal);
        }
    }
}

/// Forward the termination signals to the supervised child.
///
/// The signals are the ones a supervisor or an operator uses to stop a
/// worker: `SIGTERM`, `SIGINT`, `SIGHUP` and `SIGQUIT`. The process that
/// forwards them stays alive until its child ends, so the child can shut down
/// in an orderly way. `SIGKILL` cannot be caught and always reaches every
/// level of the chain.
#[cfg(target_os = "linux")]
fn forward_signals_to(child: i32) {
    FORWARD_TARGET.store(child, Ordering::SeqCst);
    let handler = forward_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT] {
        // SAFETY: the handler only calls `kill`.
        unsafe {
            libc::signal(signal, handler);
        }
    }
}

/// Forward the termination signals to the supervised child.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn forward_signals_to(_child: i32) {}

/// Ask the kernel to kill this process when its parent dies.
///
/// The parent-death signal is cleared for the child of a `fork`, so every
/// level of the chain arms it again after its own `fork`.
///
/// The namespace init uses this function: its parent lives in the parent PID
/// namespace, so the parent cannot be compared after the `fork` and the
/// classic "the parent died before the signal was armed" race cannot be
/// closed here. The window is a few instructions wide, and the supervisor
/// kills the whole process group, so the realistic paths are covered.
#[cfg(target_os = "linux")]
pub fn arm_parent_death_signal() {
    // SAFETY: `prctl` with `PR_SET_PDEATHSIG` only records a signal.
    unsafe {
        libc::syscall(
            libc::SYS_prctl,
            PR_SET_PDEATHSIG,
            libc::SIGKILL,
            0 as libc::c_long,
            0 as libc::c_long,
            0 as libc::c_long,
        );
    }
}

/// Ask the kernel to kill this process when its parent dies.
#[cfg(not(target_os = "linux"))]
pub fn arm_parent_death_signal() {}

/// Arm the parent-death signal and verify that the namespace init is alive.
///
/// The worker is a child of the namespace init, which is PID 1 of the
/// namespace, so the parent *is* comparable after the `fork` and the race
/// between the `fork` and the `prctl` is closed.
#[cfg(target_os = "linux")]
fn arm_parent_death_signal_within_namespace() {
    arm_parent_death_signal();
    // SAFETY: `getppid` has no memory-safety preconditions.
    if unsafe { libc::getppid() } != 1 {
        // The namespace init died before the signal was armed; the kernel
        // tears the namespace down, so this process ends with it.
        // SAFETY: the process ends here.
        unsafe {
            libc::_exit(ISOLATION_FAILURE_EXIT);
        }
    }
}

/// Supervise the namespace init and end exactly as the worker ended.
///
/// This is the process the accepted supervisor started. It never returns.
#[cfg(target_os = "linux")]
fn supervise_namespace_init(child: i32, channel: &StatusChannel) -> ! {
    forward_signals_to(child);
    let mut status: libc::c_int = 0;
    loop {
        // SAFETY: the init is the child of this process.
        let waited = unsafe { libc::waitpid(child, &mut status, 0) };
        if waited < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            // The init cannot be waited for; the process ends without a
            // status of its own.
            // SAFETY: the process ends here.
            unsafe {
                libc::_exit(1);
            }
        }
        break;
    }
    // The init reported the status of the worker, unless it was killed
    // before it could report one.
    match channel.read_status() {
        Some(worker) => mirror_status(worker),
        None => mirror_status(status),
    }
}

/// Run the worker inside the jail and end exactly as the worker ended.
///
/// The caller is PID 1 of the namespace: the worker is its child, so an
/// orphan the worker leaves behind is reparented to PID 1 and reaped here.
#[cfg(target_os = "linux")]
fn supervise_worker(command: &WorkerCommand, channel: Option<&StatusChannel>) -> i32 {
    // SAFETY: the init is a single-threaded process.
    let child = unsafe { libc::fork() };
    if child < 0 {
        return setup_failure(&crate::isolation::libc_error("fork"));
    }
    if child == 0 {
        // The worker.
        arm_parent_death_signal_within_namespace();
        exec_worker(command);
    }
    forward_signals_to(child);
    loop {
        let mut status: libc::c_int = 0;
        // SAFETY: the init waits for its own children.
        let pid = unsafe { libc::waitpid(-1, &mut status, 0) };
        if pid < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            // No child is left; the worker vanished without a status.
            return 1;
        }
        if pid == child {
            if let Some(channel) = channel {
                // The init cannot repeat the death of the worker, so it
                // reports it to the parent of the namespace.
                channel.write_status(status);
                return mirror_status_as_init(status);
            }
            // Without a PID namespace the caller is not the init of one, so it
            // can repeat the death itself.
            mirror_status(status);
        }
        // An orphan was reaped; the init keeps waiting for the worker.
    }
}

/// Supervise the namespace init and end exactly as the worker ended.
#[cfg(not(target_os = "linux"))]
fn supervise_namespace_init(_child: i32, _channel: &StatusChannel) -> ! {
    std::process::exit(1);
}

/// Run the worker inside the jail and end exactly as the worker ended.
#[cfg(not(target_os = "linux"))]
fn supervise_worker(_command: &WorkerCommand, _channel: Option<&StatusChannel>) -> i32 {
    setup_failure(&SandboxSetupError::new(
        "the strict profile requires Linux namespaces",
    ))
}

/// End the namespace init exactly as the worker ended.
///
/// The init of a PID namespace cannot be killed by a fatal signal that is sent
/// from inside the namespace, so the conventional status is used for the exit
/// code and the signal is reported through the status channel.
#[cfg(target_os = "linux")]
fn mirror_status_as_init(status: libc::c_int) -> i32 {
    if libc::WIFSIGNALED(status) {
        let signal = libc::WTERMSIG(status);
        // SAFETY: the process ends here.
        unsafe {
            libc::_exit(128 + signal);
        }
    }
    let code = libc::WEXITSTATUS(status) & 0xff;
    // SAFETY: the process ends here.
    unsafe {
        libc::_exit(code);
    }
}

/// End this process exactly as the status describes.
///
/// A signalled child is repeated: the process dies from the same signal, so
/// the supervisor observes the same death it observed when it launched the
/// worker directly. A normal exit keeps the exit code of the child.
#[cfg(target_os = "linux")]
fn mirror_status(status: libc::c_int) -> ! {
    if libc::WIFSIGNALED(status) {
        let signal = libc::WTERMSIG(status);
        if !stops_the_process(signal) {
            // SAFETY: the default disposition of the signal terminates the
            // process, exactly like it terminated the child.
            unsafe {
                libc::signal(signal, libc::SIG_DFL);
                libc::raise(signal);
            }
        }
        // The signal was blocked or ignored: the conventional status is used.
        // SAFETY: the process ends here.
        unsafe {
            libc::_exit(128 + signal);
        }
    }
    let code = libc::WEXITSTATUS(status) & 0xff;
    // SAFETY: the process ends here.
    unsafe {
        libc::_exit(code);
    }
}

/// Whether the signal stops the process instead of terminating it.
///
/// A stopped signal must not be repeated: the process would stop instead of
/// ending, and the supervisor would wait for a process that never exits.
#[cfg(target_os = "linux")]
fn stops_the_process(signal: libc::c_int) -> bool {
    matches!(
        signal,
        libc::SIGSTOP | libc::SIGTSTP | libc::SIGTTIN | libc::SIGTTOU
    )
}

/// End this process exactly as the status describes.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn mirror_status(_status: i32) -> ! {
    std::process::exit(1);
}

/// Replace the calling process with the worker.
#[cfg(target_os = "linux")]
fn exec_worker(command: &WorkerCommand) -> ! {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let executable = match CString::new(command.executable.as_os_str().as_bytes()) {
        Ok(value) => value,
        Err(_) => exec_failure("the worker executable contains a null byte"),
    };
    let mut arguments: Vec<CString> = Vec::with_capacity(command.arguments.len() + 1);
    arguments.push(executable.clone());
    for argument in &command.arguments {
        match CString::new(argument.as_bytes()) {
            Ok(value) => arguments.push(value),
            Err(_) => exec_failure("a worker argument contains a null byte"),
        }
    }
    let mut argument_pointers: Vec<*const libc::c_char> =
        arguments.iter().map(|value| value.as_ptr()).collect();
    argument_pointers.push(std::ptr::null());

    let environment: Vec<CString> = std::env::vars_os()
        .filter_map(|(key, value)| {
            let mut entry = key.as_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            CString::new(entry).ok()
        })
        .collect();
    let mut environment_pointers: Vec<*const libc::c_char> =
        environment.iter().map(|value| value.as_ptr()).collect();
    environment_pointers.push(std::ptr::null());

    // SAFETY: every pointer refers to a live NUL-terminated C string.
    unsafe {
        libc::execve(
            executable.as_ptr(),
            argument_pointers.as_ptr(),
            environment_pointers.as_ptr(),
        );
    }
    let error = std::io::Error::last_os_error();
    exec_failure(&format!(
        "the worker could not be executed: execve failed with errno {} ({})",
        error.raw_os_error().unwrap_or(0),
        crate::isolation::os_reason(&error)
    ))
}

/// Report a failed execution and end the process.
#[cfg(target_os = "linux")]
fn exec_failure(message: &str) -> ! {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{message}");
    let _ = std::io::stderr().flush();
    // SAFETY: the process ends here.
    unsafe {
        libc::_exit(EXEC_FAILURE_EXIT);
    }
}

/// The text of an argument.
fn text_of(argument: &OsString) -> String {
    argument.to_string_lossy().into_owned()
}
