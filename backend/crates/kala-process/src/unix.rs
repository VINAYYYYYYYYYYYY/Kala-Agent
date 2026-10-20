//! Unix process launch and supervision.
//!
//! A raw `pipe2`/`fork`/`execve` path is used instead of `std::process` so the
//! parent controls process groups, descriptor inheritance, the exec handshake
//! and unreaped exit observation. The child side of `fork` performs only
//! async-signal-safe work: `dup2`, `fcntl`, `close`, `setrlimit`, `signal`,
//! `sigprocmask`, raw sandbox syscalls and `execve`. No allocation, formatting,
//! environment access or locking happens there, which is required because the
//! embedding process may be multithreaded.

use std::ffi::{CString, OsStr};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::environment::ChildEnvironment;
use crate::error::{ProcessError, SandboxSetupError};
use crate::limits::ProcessLimits;
use crate::outcome::{
    CancelStage, CapturedOutput, ExitStatus, OutputStream, ProcessEnd, ProcessOutcome,
    TimeoutStage, TreeCleanup,
};
use crate::process::{CancelToken, ProcessSpec};
use crate::sandbox::{ISOLATION_FAILURE_EXIT, sanitize_child_stderr};
use crate::sandbox_plan::PreparedSandbox;

const SUCCESS_BYTE: u8 = 0x01;
const FAILURE_BYTE: u8 = 0x00;
const STAGE_SETPGID: u32 = 0xFFFF_FF01;
const STAGE_DUP: u32 = 0xFFFF_FF02;
const STAGE_SIGNAL: u32 = 0xFFFF_FF03;
const STAGE_WORKDIR: u32 = 0xFFFF_FF04;
const STAGE_EXEC: u32 = 0xFFFF_FF05;
const READ_CHUNK: usize = 32 * 1024;
/// Maximum bytes one `read_capture` call will consume before returning to the
/// supervisor loop. Without this budget a continuously ready producer could
/// starve deadline, cancellation and output-cap decisions.
const READ_BUDGET_BYTES: usize = 8 * READ_CHUNK;
/// Maximum stdin bytes one `pump` call will write before the supervisor loop
/// re-checks cancellation, the bootstrap deadline and the wall deadline.
const WRITE_BUDGET_BYTES: usize = 8 * READ_CHUNK;
const POLL_SLICE_MS: u64 = 20;

/// Supervise one Unix process.
pub(crate) fn supervise(
    spec: &ProcessSpec,
    stdin: &[u8],
    cancel: &CancelToken,
) -> Result<ProcessOutcome, ProcessError> {
    let prepared = match &spec.sandbox {
        Some(sandbox) => Some(PreparedSandbox::new(sandbox).map_err(ProcessError::SandboxSetup)?),
        None => None,
    };
    let started = Instant::now();
    let resources = ResourcePlan::new(&spec.limits);
    let mut child = spawn(spec, prepared.as_ref(), resources)?;
    let bootstrap_deadline = started + spec.limits.bootstrap_timeout;
    let wall_deadline = started + spec.limits.wall_timeout;
    let handshake_deadline = bootstrap_deadline.min(wall_deadline);
    let timeout_stage = if bootstrap_deadline <= wall_deadline {
        TimeoutStage::Bootstrap
    } else {
        TimeoutStage::WallClock
    };
    let mut stdout = CapturedOutput::new();
    let mut stderr = CapturedOutput::new();
    // The parent ends must be non-blocking before the handshake can fail:
    // cleanup in `finish` must never block on a pipe held by a live child or
    // descendant, even on an early launch error.
    if let Err(error) = child.prepare_io() {
        let _ = child.finish(
            &mut stdout,
            &mut stderr,
            spec.limits.max_stdout_bytes,
            spec.limits.max_stderr_bytes,
            spec.limits.termination_timeout,
        );
        return Err(error);
    }
    match child.handshake(
        handshake_deadline,
        cancel,
        timeout_stage,
        prepared.as_ref(),
        &spec.executable,
    ) {
        Ok(()) => {}
        Err(HandshakeFailure::End(end)) => {
            let cleanup = child.finish(
                &mut stdout,
                &mut stderr,
                spec.limits.max_stdout_bytes,
                spec.limits.max_stderr_bytes,
                spec.limits.termination_timeout,
            );
            if !cleanup.direct_reaped {
                return Err(ProcessError::Cleanup(
                    "the worker could not be reaped after a bootstrap failure".to_owned(),
                ));
            }
            return Ok(outcome(
                end,
                child.exit_status,
                stdout,
                stderr,
                started.elapsed(),
                child.pid,
                cleanup,
                &spec.working_directory,
            ));
        }
        Err(HandshakeFailure::Error(error)) => {
            let _ = child.finish(
                &mut stdout,
                &mut stderr,
                spec.limits.max_stdout_bytes,
                spec.limits.max_stderr_bytes,
                spec.limits.termination_timeout,
            );
            return Err(error);
        }
    }
    let end = match child.supervise_loop(
        stdin,
        cancel,
        bootstrap_deadline,
        wall_deadline,
        &spec.limits,
        &mut stdout,
        &mut stderr,
    ) {
        Ok(end) => end,
        Err(error) => {
            let _ = child.finish(
                &mut stdout,
                &mut stderr,
                spec.limits.max_stdout_bytes,
                spec.limits.max_stderr_bytes,
                spec.limits.termination_timeout,
            );
            return Err(error);
        }
    };
    let cleanup = child.finish(
        &mut stdout,
        &mut stderr,
        spec.limits.max_stdout_bytes,
        spec.limits.max_stderr_bytes,
        spec.limits.termination_timeout,
    );
    if !cleanup.direct_reaped {
        return Err(ProcessError::Cleanup(
            "the worker could not be reaped within the termination deadline".to_owned(),
        ));
    }
    let end = if matches!(end, ProcessEnd::Exited(_))
        && stdout.truncated()
        && stdout.len() >= spec.limits.max_stdout_bytes
    {
        ProcessEnd::OutputLimitExceeded(OutputStream::Stdout)
    } else {
        end
    };
    Ok(outcome(
        end,
        child.exit_status,
        stdout,
        stderr,
        started.elapsed(),
        child.pid,
        cleanup,
        &spec.working_directory,
    ))
}

#[allow(clippy::too_many_arguments)]
fn outcome(
    end: ProcessEnd,
    exit: Option<ExitStatus>,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
    elapsed: Duration,
    pid: libc::pid_t,
    cleanup: TreeCleanup,
    workspace: &Path,
) -> ProcessOutcome {
    let stderr_summary =
        sanitize_child_stderr(stderr.as_bytes(), Some(&workspace.to_string_lossy()));
    ProcessOutcome {
        end,
        exit,
        stdout,
        stderr,
        stderr_summary,
        elapsed,
        process_id: Some(pid as u32),
        cleanup,
    }
}

enum HandshakeFailure {
    End(ProcessEnd),
    Error(ProcessError),
}

struct Pipe {
    read: Option<RawFd>,
    write: Option<RawFd>,
}

impl Pipe {
    fn new() -> io::Result<Self> {
        let mut descriptors = [0 as RawFd; 2];
        // SAFETY: the array is a valid output location.
        let result = unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        let read = match duplicate_above_stdio(descriptors[0]) {
            Ok(read) => read,
            Err(error) => {
                close_fd(descriptors[0]);
                close_fd(descriptors[1]);
                return Err(error);
            }
        };
        let write = match duplicate_above_stdio(descriptors[1]) {
            Ok(write) => write,
            Err(error) => {
                if read != descriptors[0] {
                    close_fd(read);
                }
                close_fd(descriptors[0]);
                close_fd(descriptors[1]);
                return Err(error);
            }
        };
        if read != descriptors[0] {
            close_fd(descriptors[0]);
        }
        if write != descriptors[1] {
            close_fd(descriptors[1]);
        }
        Ok(Self {
            read: Some(read),
            write: Some(write),
        })
    }

    fn read(&mut self) -> RawFd {
        self.read.take().expect("pipe read end already taken")
    }

    fn write(&mut self) -> RawFd {
        self.write.take().expect("pipe write end already taken")
    }

    fn close_read(&mut self) {
        if let Some(descriptor) = self.read.take() {
            close_fd(descriptor);
        }
    }

    fn close_write(&mut self) {
        if let Some(descriptor) = self.write.take() {
            close_fd(descriptor);
        }
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        self.close_read();
        self.close_write();
    }
}

#[derive(Clone, Copy)]
struct ResourcePlan {
    enabled: bool,
    cpu_seconds: u64,
    file_size: u64,
    max_fd: i64,
}

impl ResourcePlan {
    fn new(limits: &ProcessLimits) -> Self {
        let cpu_seconds = limits.wall_timeout.as_secs().saturating_add(1).max(1);
        // SAFETY: sysconf has no memory-safety preconditions.
        let max_fd = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
        Self {
            enabled: limits.enforce_resource_limits,
            cpu_seconds,
            file_size: limits.max_artifact_bytes,
            max_fd: if max_fd <= 0 { 65_536 } else { max_fd },
        }
    }
}

fn spawn(
    spec: &ProcessSpec,
    sandbox: Option<&PreparedSandbox>,
    resources: ResourcePlan,
) -> Result<UnixChild, ProcessError> {
    let executable = os_cstring(&spec.executable, "executable")?;
    let working_directory = os_cstring(&spec.working_directory, "working directory")?;
    let mut argv_strings: Vec<CString> = Vec::with_capacity(spec.arguments.len().saturating_add(1));
    argv_strings.push(executable.clone());
    for argument in &spec.arguments {
        argv_strings.push(os_cstring(argument, "argument")?);
    }
    let mut argv: Vec<*const libc::c_char> =
        argv_strings.iter().map(|value| value.as_ptr()).collect();
    argv.push(std::ptr::null());
    let environment = environment_strings(&spec.environment)?;
    let mut envp: Vec<*const libc::c_char> =
        environment.iter().map(|value| value.as_ptr()).collect();
    envp.push(std::ptr::null());

    let mut stdin_pipe = Pipe::new().map_err(|error| spawn_io_error(&spec.executable, error))?;
    let mut stdout_pipe = Pipe::new().map_err(|error| spawn_io_error(&spec.executable, error))?;
    let mut stderr_pipe = Pipe::new().map_err(|error| spawn_io_error(&spec.executable, error))?;
    let mut error_pipe = Pipe::new().map_err(|error| spawn_io_error(&spec.executable, error))?;

    let child_stdin = stdin_pipe.read();
    let child_stdout = stdout_pipe.write();
    let child_stderr = stderr_pipe.write();
    let child_error = error_pipe.write();
    let parent_stdin = stdin_pipe.write();
    let parent_stdout = stdout_pipe.read();
    let parent_stderr = stderr_pipe.read();
    let parent_error = error_pipe.read();

    // SAFETY: the child branch only performs raw system calls before exec.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        let error = io::Error::last_os_error();
        for descriptor in [
            child_stdin,
            child_stdout,
            child_stderr,
            child_error,
            parent_stdin,
            parent_stdout,
            parent_stderr,
            parent_error,
        ] {
            close_fd(descriptor);
        }
        return Err(spawn_io_error(&spec.executable, error));
    }
    if pid == 0 {
        // Child. Keep the pointers alive; this function never returns.
        unsafe {
            child_main(
                child_stdin,
                child_stdout,
                child_stderr,
                child_error,
                &executable,
                &argv,
                &envp,
                &working_directory,
                sandbox,
                resources,
            );
        }
    }
    // Parent. The child-side descriptors were taken out of the Pipe owners, so
    // they must be closed explicitly here; otherwise the parent would keep the
    // bootstrap writer (and the stdout/stderr writers) open forever.
    close_fd(child_stdin);
    close_fd(child_stdout);
    close_fd(child_stderr);
    close_fd(child_error);
    Ok(UnixChild {
        pid,
        stdin_write: Some(parent_stdin),
        stdout_read: Some(parent_stdout),
        stderr_read: Some(parent_stderr),
        error_read: Some(parent_error),
        reaped: false,
        exit_status: None,
    })
}

fn duplicate_above_stdio(descriptor: RawFd) -> io::Result<RawFd> {
    if descriptor > 2 {
        return Ok(descriptor);
    }
    // SAFETY: fcntl duplicates our descriptor with close-on-exec set.
    let duplicate = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(duplicate)
}

fn spawn_io_error(path: &Path, error: io::Error) -> ProcessError {
    ProcessError::spawn(path, error.to_string())
}

fn environment_strings(environment: &ChildEnvironment) -> Result<Vec<CString>, ProcessError> {
    environment.validate()?;
    environment
        .entries()
        .iter()
        .map(|entry| {
            CString::new(format!("{}={}", entry.key, entry.value))
                .map_err(|_| ProcessError::Environment(crate::EnvironmentError::NullValue))
        })
        .collect()
}

fn os_cstring(value: impl AsRef<OsStr>, label: &str) -> Result<CString, ProcessError> {
    CString::new(value.as_ref().as_bytes())
        .map_err(|_| ProcessError::InvalidSpec(format!("the {label} contains a null byte")))
}

struct UnixChild {
    pid: libc::pid_t,
    stdin_write: Option<RawFd>,
    stdout_read: Option<RawFd>,
    stderr_read: Option<RawFd>,
    error_read: Option<RawFd>,
    reaped: bool,
    exit_status: Option<ExitStatus>,
}

impl Drop for UnixChild {
    fn drop(&mut self) {
        if let Some(descriptor) = self.error_read.take() {
            close_fd(descriptor);
        }
        if let Some(descriptor) = self.stdin_write.take() {
            close_fd(descriptor);
        }
        if let Some(descriptor) = self.stdout_read.take() {
            close_fd(descriptor);
        }
        if let Some(descriptor) = self.stderr_read.take() {
            close_fd(descriptor);
        }
        if !self.reaped {
            // Last-resort cleanup: never leave a running worker behind.
            let _ = kill_tree(self.pid);
            // SAFETY: waitpid is called for our own unreaped child.
            let _ = unsafe { libc::waitpid(self.pid, std::ptr::null_mut(), libc::WNOHANG) };
        }
    }
}

impl UnixChild {
    /// Make every parent-side stdio descriptor non-blocking. This runs before
    /// the bootstrap handshake so `finish` can always drain bounded data
    /// without blocking, including on handshake failures.
    fn prepare_io(&mut self) -> Result<(), ProcessError> {
        if let Some(descriptor) = self.stdin_write {
            set_nonblocking(descriptor).map_err(|error| {
                ProcessError::Io(format!("stdin could not be made non-blocking: {error}"))
            })?;
        }
        if let Some(descriptor) = self.stdout_read {
            set_nonblocking(descriptor)
                .map_err(|error| ProcessError::Io(format!("stdout pipe failed: {error}")))?;
        }
        if let Some(descriptor) = self.stderr_read {
            set_nonblocking(descriptor)
                .map_err(|error| ProcessError::Io(format!("stderr pipe failed: {error}")))?;
        }
        Ok(())
    }

    fn handshake(
        &mut self,
        deadline: Instant,
        cancel: &CancelToken,
        timeout_stage: TimeoutStage,
        prepared: Option<&PreparedSandbox>,
        executable: &Path,
    ) -> Result<(), HandshakeFailure> {
        let descriptor = match self.error_read.take() {
            Some(descriptor) => descriptor,
            None => {
                return Err(HandshakeFailure::Error(ProcessError::Internal(
                    "the bootstrap descriptor is missing".to_owned(),
                )));
            }
        };
        let mut buffer = [0u8; 64];
        let mut received = 0usize;
        loop {
            if cancel.is_cancelled() {
                close_fd(descriptor);
                return Err(HandshakeFailure::End(ProcessEnd::Cancelled(
                    CancelStage::Bootstrap,
                )));
            }
            let now = Instant::now();
            if now >= deadline {
                close_fd(descriptor);
                return Err(HandshakeFailure::End(ProcessEnd::TimedOut(timeout_stage)));
            }
            let remaining = deadline.saturating_duration_since(now);
            let timeout = remaining.min(Duration::from_millis(POLL_SLICE_MS));
            let mut pollfd = libc::pollfd {
                fd: descriptor,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd.
            let ready = unsafe { libc::poll(&mut pollfd, 1, millis(timeout)) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                close_fd(descriptor);
                return Err(HandshakeFailure::Error(ProcessError::Bootstrap(format!(
                    "the bootstrap pipe failed: {error}"
                ))));
            }
            if ready == 0 {
                continue;
            }
            // SAFETY: the buffer is valid and the descriptor is ours.
            let count = unsafe {
                libc::read(
                    descriptor,
                    buffer[received..].as_mut_ptr().cast(),
                    buffer.len() - received,
                )
            };
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                close_fd(descriptor);
                return Err(HandshakeFailure::Error(ProcessError::Bootstrap(format!(
                    "the bootstrap pipe could not be read: {error}"
                ))));
            }
            if count == 0 {
                break;
            }
            received += count as usize;
            if received == buffer.len() {
                break;
            }
        }
        close_fd(descriptor);
        parse_handshake(&buffer[..received], prepared, executable)
    }

    #[allow(clippy::too_many_arguments)]
    fn supervise_loop(
        &mut self,
        stdin: &[u8],
        cancel: &CancelToken,
        bootstrap_deadline: Instant,
        wall_deadline: Instant,
        limits: &ProcessLimits,
        stdout: &mut CapturedOutput,
        stderr: &mut CapturedOutput,
    ) -> Result<ProcessEnd, ProcessError> {
        // `prepare_io` already made these descriptors non-blocking; taking the
        // values here keeps ownership in the loop and clears the fields so
        // `finish` never touches a descriptor the loop still uses.
        let stdin_fd = self.stdin_write.take();
        let stdout_fd = self.stdout_read.expect("stdout pipe present");
        let stderr_fd = self.stderr_read.expect("stderr pipe present");
        let mut stdin_state = StdinState {
            descriptor: stdin_fd,
            offset: 0,
        };
        let mut stdout_open = true;
        let mut stderr_open = true;
        let mut failure: Option<ProcessEnd> = None;
        let mut observed_exit: Option<ExitStatus> = None;
        loop {
            if let Some(status) = observe_exit(self.pid)? {
                observed_exit = Some(status);
                break;
            }
            if stdout_open {
                match read_capture(stdout_fd, stdout, limits.max_stdout_bytes, false) {
                    Ok(ReadState::Eof) => {
                        stdout_open = false;
                        if let Some(descriptor) = self.stdout_read.take() {
                            close_fd(descriptor);
                        }
                    }
                    Ok(ReadState::WouldBlock | ReadState::Data) => {}
                    Err(error) => {
                        return Err(ProcessError::Io(format!("stdout read failed: {error}")));
                    }
                }
                if stdout.truncated() && stdout.len() >= limits.max_stdout_bytes {
                    failure = Some(ProcessEnd::OutputLimitExceeded(OutputStream::Stdout));
                    break;
                }
            }
            if stderr_open {
                match read_capture(stderr_fd, stderr, limits.max_stderr_bytes, true) {
                    Ok(ReadState::Eof) => {
                        stderr_open = false;
                        if let Some(descriptor) = self.stderr_read.take() {
                            close_fd(descriptor);
                        }
                    }
                    Ok(ReadState::WouldBlock | ReadState::Data) => {}
                    Err(error) => {
                        return Err(ProcessError::Io(format!("stderr read failed: {error}")));
                    }
                }
            }
            stdin_state.pump(stdin);
            if cancel.is_cancelled() {
                failure = Some(ProcessEnd::Cancelled(CancelStage::Running));
                break;
            }
            let now = Instant::now();
            // The bootstrap deadline also bounds delivery and closure of
            // stdin, not just the exec handshake. Without this check a worker
            // that never reads stdin could run until the wall deadline. Use
            // the same stage precedence as the handshake when both expire.
            if stdin_state.is_pending()
                && now >= bootstrap_deadline
                && bootstrap_deadline <= wall_deadline
            {
                failure = Some(ProcessEnd::TimedOut(TimeoutStage::Bootstrap));
                break;
            }
            if now >= wall_deadline {
                failure = Some(ProcessEnd::TimedOut(TimeoutStage::WallClock));
                break;
            }
            let mut pollfds: Vec<libc::pollfd> = Vec::with_capacity(3);
            if stdout_open {
                pollfds.push(libc::pollfd {
                    fd: stdout_fd,
                    events: libc::POLLIN,
                    revents: 0,
                });
            }
            if stderr_open {
                pollfds.push(libc::pollfd {
                    fd: stderr_fd,
                    events: libc::POLLIN,
                    revents: 0,
                });
            }
            if stdin_state.is_pending() {
                if let Some(descriptor) = stdin_state.descriptor {
                    pollfds.push(libc::pollfd {
                        fd: descriptor,
                        events: libc::POLLOUT,
                        revents: 0,
                    });
                }
            }
            let deadline = if stdin_state.is_pending() {
                wall_deadline.min(bootstrap_deadline)
            } else {
                wall_deadline
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = remaining.min(Duration::from_millis(POLL_SLICE_MS));
            // SAFETY: the vector describes valid descriptors owned above.
            let ready = unsafe {
                libc::poll(
                    pollfds.as_mut_ptr(),
                    pollfds.len() as libc::nfds_t,
                    millis(timeout),
                )
            };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(ProcessError::Io(format!("poll failed: {error}")));
            }
        }
        if let Some(failure) = failure {
            return Ok(failure);
        }
        match observed_exit {
            Some(status) => {
                self.exit_status = Some(status);
                Ok(ProcessEnd::Exited(status))
            }
            None => Err(ProcessError::Wait(
                "the worker ended without an observable status".to_owned(),
            )),
        }
    }

    fn finish(
        &mut self,
        stdout: &mut CapturedOutput,
        stderr: &mut CapturedOutput,
        stdout_limit: usize,
        stderr_limit: usize,
        termination: Duration,
    ) -> TreeCleanup {
        let deadline = Instant::now() + termination;
        if let Some(descriptor) = self.stdin_write.take() {
            close_fd(descriptor);
        }
        // Defensive: `prepare_io` already did this, but an early failure during
        // preparation must not leave a blocking descriptor for cleanup to read.
        for descriptor in [self.stdout_read, self.stderr_read].into_iter().flatten() {
            if set_nonblocking(descriptor).is_err() {
                close_fd(descriptor);
                if self.stdout_read == Some(descriptor) {
                    self.stdout_read = None;
                }
                if self.stderr_read == Some(descriptor) {
                    self.stderr_read = None;
                }
            }
        }
        let killed = kill_tree(self.pid);
        let mut stdout_open = self.stdout_read.is_some();
        let mut stderr_open = self.stderr_read.is_some();
        while Instant::now() < deadline {
            if stdout_open {
                if let Some(descriptor) = self.stdout_read {
                    match read_capture(descriptor, stdout, stdout_limit, false) {
                        Ok(ReadState::Eof) => {
                            stdout_open = false;
                            if let Some(descriptor) = self.stdout_read.take() {
                                close_fd(descriptor);
                            }
                        }
                        Ok(ReadState::WouldBlock | ReadState::Data) => {}
                        Err(_) => {
                            close_fd(descriptor);
                            self.stdout_read = None;
                            stdout_open = false;
                        }
                    }
                } else {
                    stdout_open = false;
                }
            }
            if stderr_open {
                if let Some(descriptor) = self.stderr_read {
                    match read_capture(descriptor, stderr, stderr_limit, true) {
                        Ok(ReadState::Eof) => {
                            stderr_open = false;
                            if let Some(descriptor) = self.stderr_read.take() {
                                close_fd(descriptor);
                            }
                        }
                        Ok(ReadState::WouldBlock | ReadState::Data) => {}
                        Err(_) => {
                            close_fd(descriptor);
                            self.stderr_read = None;
                            stderr_open = false;
                        }
                    }
                } else {
                    stderr_open = false;
                }
            }
            if !stdout_open && !stderr_open {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if stdout_open {
            if let Some(descriptor) = self.stdout_read.take() {
                close_fd(descriptor);
            }
        }
        if stderr_open {
            if let Some(descriptor) = self.stderr_read.take() {
                close_fd(descriptor);
            }
        }
        let mut reaped = false;
        while Instant::now() < deadline {
            let mut status: libc::c_int = 0;
            // SAFETY: waitpid targets our own child.
            let result = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if result == self.pid {
                self.exit_status = parse_wait_status(status);
                reaped = true;
                self.reaped = true;
                break;
            }
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ECHILD) {
                    reaped = true;
                    self.reaped = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if !reaped {
            let _ = kill_tree(self.pid);
            std::thread::sleep(Duration::from_millis(2));
            let mut status: libc::c_int = 0;
            // SAFETY: final non-blocking reap attempt.
            let result = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if result == self.pid {
                self.exit_status = parse_wait_status(status);
                reaped = true;
                self.reaped = true;
            }
        }
        let mut tree_empty = false;
        while Instant::now() < deadline {
            if !process_group_exists(self.pid) {
                tree_empty = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if !tree_empty {
            tree_empty = !process_group_exists(self.pid);
        }
        TreeCleanup {
            direct_reaped: reaped,
            tree_terminated: killed || tree_empty,
            tree_empty,
            pipes_drained: !stdout_open && !stderr_open,
            termination_timed_out: Instant::now() >= deadline,
        }
    }
}

struct StdinState {
    descriptor: Option<RawFd>,
    offset: usize,
}

impl Drop for StdinState {
    fn drop(&mut self) {
        if let Some(descriptor) = self.descriptor.take() {
            close_fd(descriptor);
        }
    }
}

impl StdinState {
    fn is_pending(&self) -> bool {
        self.descriptor.is_some()
    }

    fn pump(&mut self, payload: &[u8]) {
        let descriptor = match self.descriptor {
            Some(descriptor) => descriptor,
            None => return,
        };
        if self.offset >= payload.len() {
            close_fd(descriptor);
            self.descriptor = None;
            return;
        }
        // Byte-budgeted so a fast reader cannot keep this loop from re-checking
        // cancellation and the bootstrap/wall deadlines.
        let budget_end = self
            .offset
            .saturating_add(WRITE_BUDGET_BYTES)
            .min(payload.len());
        loop {
            if self.offset >= payload.len() {
                close_fd(descriptor);
                self.descriptor = None;
                return;
            }
            if self.offset >= budget_end {
                return;
            }
            // SAFETY: the payload offset is in bounds and the descriptor is ours.
            let written = unsafe {
                libc::write(
                    descriptor,
                    payload[self.offset..].as_ptr().cast(),
                    budget_end - self.offset,
                )
            };
            if written > 0 {
                self.offset += written as usize;
                continue;
            }
            if written == 0 {
                close_fd(descriptor);
                self.descriptor = None;
                return;
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EINTR) => continue,
                Some(libc::EAGAIN) => return,
                Some(libc::EPIPE) | Some(libc::EBADF) => {
                    close_fd(descriptor);
                    self.descriptor = None;
                    return;
                }
                _ => {
                    close_fd(descriptor);
                    self.descriptor = None;
                    return;
                }
            }
        }
    }
}

enum ReadState {
    Data,
    WouldBlock,
    Eof,
}

fn read_capture(
    descriptor: RawFd,
    capture: &mut CapturedOutput,
    limit: usize,
    discard_after_limit: bool,
) -> io::Result<ReadState> {
    let mut buffer = [0u8; READ_CHUNK];
    let mut read_any = false;
    let mut budget = READ_BUDGET_BYTES;
    loop {
        if budget == 0 {
            return Ok(ReadState::Data);
        }
        let request = buffer.len().min(budget);
        // SAFETY: the stack buffer is valid and the descriptor is owned by the
        // caller.
        let count = unsafe { libc::read(descriptor, buffer.as_mut_ptr().cast(), request) };
        if count > 0 {
            read_any = true;
            budget -= count as usize;
            let chunk = &buffer[..count as usize];
            if discard_after_limit && capture.len() >= limit {
                capture.discard(chunk.len());
            } else {
                capture.push(chunk, limit);
            }
            continue;
        }
        if count == 0 {
            return Ok(ReadState::Eof);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EAGAIN) => {
                return Ok(if read_any {
                    ReadState::Data
                } else {
                    ReadState::WouldBlock
                });
            }
            _ => return Err(error),
        }
    }
}

fn kill_tree(pid: libc::pid_t) -> bool {
    // The direct child is unreaped here, so its pid (and therefore its pgid)
    // cannot have been reused. Signal both forms: the group for descendants,
    // the pid for the window before the child called setpgid.
    // SAFETY: kill has no memory-safety preconditions.
    let direct = unsafe { libc::kill(pid, libc::SIGKILL) };
    let direct_error = io::Error::last_os_error().raw_os_error();
    let group = unsafe { libc::kill(-pid, libc::SIGKILL) };
    let group_error = io::Error::last_os_error().raw_os_error();
    let direct_ok = direct == 0 || direct_error == Some(libc::ESRCH);
    let group_ok = group == 0 || group_error == Some(libc::ESRCH);
    direct_ok && group_ok
}

fn process_group_exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only checks existence.
    let result = unsafe { libc::kill(-pid, 0) };
    if result == 0 {
        return true;
    }
    let error = io::Error::last_os_error();
    !(error.raw_os_error() == Some(libc::ESRCH))
}

fn observe_exit(pid: libc::pid_t) -> Result<Option<ExitStatus>, ProcessError> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: zeroed siginfo_t is valid and waitid writes it.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                return Err(ProcessError::Wait(
                    "the worker was reaped before its status was observed".to_owned(),
                ));
            }
            return Err(ProcessError::Wait(error.to_string()));
        }
        // SAFETY: si_pid is only meaningful for a signal-child record; waitid
        // returned success and the union is zero-initialised.
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        // SAFETY: the record is a child state change.
        let status = unsafe { info.si_status() };
        // SAFETY: as above.
        let code = info.si_code;
        match code {
            libc::CLD_EXITED => Ok(Some(ExitStatus::exited(i64::from(status)))),
            libc::CLD_KILLED | libc::CLD_DUMPED => Ok(Some(ExitStatus::signalled(status))),
            _ => Ok(None),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut status = 0;
        // SAFETY: waitpid targets our own child.
        let result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if result == 0 {
            return Ok(None);
        }
        if result < 0 {
            return Err(ProcessError::Wait(io::Error::last_os_error().to_string()));
        }
        if libc::WIFEXITED(status) {
            Ok(Some(ExitStatus::exited(i64::from(libc::WEXITSTATUS(
                status,
            )))))
        } else if libc::WIFSIGNALED(status) {
            Ok(Some(ExitStatus::signalled(libc::WTERMSIG(status))))
        } else {
            Ok(None)
        }
    }
}

fn parse_handshake(
    bytes: &[u8],
    prepared: Option<&PreparedSandbox>,
    executable: &Path,
) -> Result<(), HandshakeFailure> {
    let mut index = 0usize;
    let mut success = false;
    let mut failure: Option<(u32, i32)> = None;
    while index < bytes.len() {
        match bytes[index] {
            SUCCESS_BYTE => {
                success = true;
                index += 1;
            }
            FAILURE_BYTE => {
                if index + 9 > bytes.len() {
                    return Err(HandshakeFailure::Error(ProcessError::Bootstrap(
                        "the bootstrap record was truncated".to_owned(),
                    )));
                }
                let stage = u32::from_le_bytes([
                    bytes[index + 1],
                    bytes[index + 2],
                    bytes[index + 3],
                    bytes[index + 4],
                ]);
                let errno = i32::from_le_bytes([
                    bytes[index + 5],
                    bytes[index + 6],
                    bytes[index + 7],
                    bytes[index + 8],
                ]);
                failure = Some((stage, errno));
                index += 9;
            }
            _ => {
                return Err(HandshakeFailure::Error(ProcessError::Bootstrap(
                    "the bootstrap record was corrupt".to_owned(),
                )));
            }
        }
    }
    if let Some((stage, errno)) = failure {
        let description = stage_description(stage, prepared);
        let detail = format!(
            "{description} failed with errno {errno} ({})",
            io::Error::from_raw_os_error(errno)
        );
        return Err(HandshakeFailure::Error(if stage == STAGE_EXEC {
            ProcessError::spawn(executable, detail)
        } else {
            ProcessError::SandboxSetup(SandboxSetupError::new(detail))
        }));
    }
    if !success {
        return Err(HandshakeFailure::Error(ProcessError::Bootstrap(
            "the worker exited before it could execute".to_owned(),
        )));
    }
    Ok(())
}

fn stage_description(stage: u32, prepared: Option<&PreparedSandbox>) -> String {
    match stage {
        STAGE_SETPGID => "setpgid".to_owned(),
        STAGE_DUP => "stdio duplication".to_owned(),
        STAGE_SIGNAL => "signal reset".to_owned(),
        STAGE_WORKDIR => "chdir into the working directory".to_owned(),
        STAGE_EXEC => "execve".to_owned(),
        other => match prepared {
            Some(prepared) => format!(
                "{} (sandbox step {other})",
                prepared.step_description(other as usize)
            ),
            None => format!("sandbox step {other}"),
        },
    }
}

fn parse_wait_status(status: libc::c_int) -> Option<ExitStatus> {
    if libc::WIFEXITED(status) {
        Some(ExitStatus::exited(i64::from(libc::WEXITSTATUS(status))))
    } else if libc::WIFSIGNALED(status) {
        Some(ExitStatus::signalled(libc::WTERMSIG(status)))
    } else {
        None
    }
}

fn set_nonblocking(descriptor: RawFd) -> io::Result<()> {
    // SAFETY: fcntl operates on our descriptor.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn millis(duration: Duration) -> i32 {
    duration.as_millis().min(i32::MAX as u128) as i32
}

fn close_fd(descriptor: RawFd) {
    // SAFETY: closing an owned descriptor is safe; errors are irrelevant.
    unsafe {
        libc::close(descriptor);
    }
}

/// Post-fork child. Never returns.
///
/// # Safety
///
/// Called exactly once in a forked child before `execve`. All pointers remain
/// valid for the lifetime of the child image, and no Rust allocation or
/// locking is performed in this function or its callees.
#[allow(clippy::too_many_arguments)]
unsafe fn child_main(
    stdin_fd: RawFd,
    stdout_fd: RawFd,
    stderr_fd: RawFd,
    error_fd: RawFd,
    executable: &CString,
    argv: &[*const libc::c_char],
    envp: &[*const libc::c_char],
    working_directory: &CString,
    sandbox: Option<&PreparedSandbox>,
    resources: ResourcePlan,
) -> ! {
    // SAFETY: called in the post-fork child; every operation is a raw syscall
    // or a read of pre-built C strings.
    unsafe {
        if reset_signals() != 0 {
            child_fail(error_fd, STAGE_SIGNAL, *libc::__errno_location());
        }
        if libc::setpgid(0, 0) != 0 {
            child_fail(error_fd, STAGE_SETPGID, *libc::__errno_location());
        }
        let stdin_copy = libc::fcntl(stdin_fd, libc::F_DUPFD_CLOEXEC, 10);
        let stdout_copy = libc::fcntl(stdout_fd, libc::F_DUPFD_CLOEXEC, 10);
        let stderr_copy = libc::fcntl(stderr_fd, libc::F_DUPFD_CLOEXEC, 10);
        let error_copy = libc::fcntl(error_fd, libc::F_DUPFD_CLOEXEC, 10);
        if stdin_copy < 0 || stdout_copy < 0 || stderr_copy < 0 || error_copy < 0 {
            child_fail(error_fd, STAGE_DUP, *libc::__errno_location());
        }
        if libc::dup2(stdin_copy, 0) < 0
            || libc::dup2(stdout_copy, 1) < 0
            || libc::dup2(stderr_copy, 2) < 0
            || libc::dup2(error_copy, 3) < 0
        {
            child_fail(error_fd, STAGE_DUP, *libc::__errno_location());
        }
        close_from(4, resources.max_fd);
        if libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
            child_fail(3, STAGE_DUP, *libc::__errno_location());
        }
        apply_resource_limits(resources);
        if sandbox.is_none() && libc::chdir(working_directory.as_ptr()) != 0 {
            child_fail(3, STAGE_WORKDIR, *libc::__errno_location());
        }
        if let Some(sandbox) = sandbox {
            if let Err((index, errno)) = sandbox.apply_raw() {
                child_fail(3, index as u32, errno);
            }
        }
        if write_all(3, &[SUCCESS_BYTE]) != 0 {
            child_fail(3, STAGE_EXEC, *libc::__errno_location());
        }
        libc::execve(executable.as_ptr(), argv.as_ptr(), envp.as_ptr());
        let errno = *libc::__errno_location();
        child_fail(3, STAGE_EXEC, errno);
    }
}

unsafe fn child_fail(descriptor: RawFd, stage: u32, errno: i32) -> ! {
    let mut record = [0u8; 9];
    record[0] = FAILURE_BYTE;
    record[1..5].copy_from_slice(&stage.to_le_bytes());
    record[5..9].copy_from_slice(&errno.to_le_bytes());
    unsafe {
        write_all(descriptor, &record);
        libc::_exit(ISOLATION_FAILURE_EXIT);
    }
}

unsafe fn write_all(descriptor: RawFd, payload: &[u8]) -> i32 {
    let mut offset = 0usize;
    while offset < payload.len() {
        let written = unsafe {
            libc::write(
                descriptor,
                payload[offset..].as_ptr().cast(),
                payload.len() - offset,
            )
        };
        if written < 0 {
            let error = unsafe { *libc::__errno_location() };
            if error == libc::EINTR {
                continue;
            }
            return -1;
        }
        if written == 0 {
            return -1;
        }
        offset += written as usize;
    }
    0
}

unsafe fn reset_signals() -> i32 {
    let mut empty: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut empty);
        if libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut()) != 0 {
            return -1;
        }
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    0
}

unsafe fn apply_resource_limits(resources: ResourcePlan) {
    if !resources.enabled {
        return;
    }
    unsafe {
        libc::umask(0o077);
        set_limit(libc::RLIMIT_CORE, 0, 0);
        set_limit(
            libc::RLIMIT_CPU,
            resources.cpu_seconds,
            resources.cpu_seconds.saturating_add(1),
        );
        set_limit(libc::RLIMIT_FSIZE, resources.file_size, resources.file_size);
        set_limit(libc::RLIMIT_NOFILE, 256, 256);
    }
}

unsafe fn set_limit(resource: libc::__rlimit_resource_t, soft: u64, hard: u64) {
    let limit = libc::rlimit {
        rlim_cur: soft as libc::rlim_t,
        rlim_max: hard as libc::rlim_t,
    };
    unsafe {
        libc::setrlimit(resource, &limit);
    }
}

unsafe fn close_from(first: RawFd, max_fd: i64) {
    #[cfg(target_os = "linux")]
    {
        let result =
            unsafe { libc::syscall(libc::SYS_close_range, first as libc::c_uint, u32::MAX, 0u32) };
        if result == 0 {
            return;
        }
    }
    let mut descriptor = first;
    while i64::from(descriptor) < max_fd && descriptor < i32::MAX {
        unsafe {
            libc::close(descriptor);
        }
        descriptor += 1;
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn read_capture_returns_after_its_byte_budget_under_a_ready_producer() {
        let mut pipe = Pipe::new().expect("pipe");
        let read = pipe.read();
        let write = pipe.write();
        // The reader stays blocking: the writer keeps the pipe ready until the
        // byte budget ends the single `read_capture` call.
        let writer = std::thread::spawn(move || {
            let chunk = [b'x'; 16 * 1024];
            loop {
                // SAFETY: the descriptor belongs to this thread and the buffer
                // is valid.
                let written = unsafe { libc::write(write, chunk.as_ptr().cast(), chunk.len()) };
                if written <= 0 {
                    break;
                }
            }
            close_fd(write);
        });
        let mut capture = CapturedOutput::new();
        let state = read_capture(read, &mut capture, usize::MAX, false).expect("read");
        assert!(matches!(state, ReadState::Data));
        assert_eq!(capture.len(), READ_BUDGET_BYTES);
        close_fd(read);
        writer.join().expect("writer");
    }

    #[test]
    fn stdin_pump_returns_after_its_byte_budget_under_a_fast_reader() {
        let mut pipe = Pipe::new().expect("pipe");
        let read = pipe.read();
        let write = pipe.write();
        set_nonblocking(write).expect("non-blocking write");
        let reader = std::thread::spawn(move || {
            let mut chunk = [0u8; 16 * 1024];
            loop {
                // SAFETY: the descriptor belongs to this thread and the buffer
                // is valid.
                let count = unsafe { libc::read(read, chunk.as_mut_ptr().cast(), chunk.len()) };
                if count <= 0 {
                    break;
                }
            }
            close_fd(read);
        });
        let payload = vec![b'x'; WRITE_BUDGET_BYTES * 2];
        let mut state = StdinState {
            descriptor: Some(write),
            offset: 0,
        };
        state.pump(&payload);
        assert!(state.offset > 0);
        assert!(state.offset <= WRITE_BUDGET_BYTES);
        assert!(state.is_pending());
        if let Some(descriptor) = state.descriptor.take() {
            close_fd(descriptor);
        }
        reader.join().expect("reader");
    }
}
