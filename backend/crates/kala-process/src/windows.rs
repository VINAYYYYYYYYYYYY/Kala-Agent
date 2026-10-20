//! Windows process launch and Job Object containment.
//!
//! The worker is created **suspended**, assigned to a Job Object configured
//! with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and only then resumed. If job
//! creation, handle-list setup or assignment fails, the suspended process is
//! terminated and never resumed. The child inherits exactly the three stdio
//! handles named in `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.
//!
//! This module is cross-compiled and linted on a Linux host; **no Windows
//! runtime execution is claimed**. Nested-job behaviour is Windows 8+; when the
//! parent is already in a job the module queries that job's limit flags and
//! uses `CREATE_BREAKAWAY_FROM_JOB` only when the parent explicitly permits it.
//! Otherwise an assignment failure is reported as a containment failure rather
//! than launching an uncontrolled child.

use std::ffi::{OsStr, OsString, c_void};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED, GetLastError,
    HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::environment::ChildEnvironment;
use crate::error::ProcessError;
use crate::limits::ProcessLimits;
use crate::outcome::{
    CancelStage, CapturedOutput, ExitStatus, OutputStream, ProcessEnd, ProcessOutcome,
    TimeoutStage, TreeCleanup,
};
use crate::process::{CancelToken, ProcessSpec};
use crate::sandbox::sanitize_child_stderr;

const POLL_MS: u64 = 10;
const READ_CHUNK: usize = 32 * 1024;

/// Supervise one Windows process.
pub(crate) fn supervise(
    spec: &ProcessSpec,
    stdin: &[u8],
    cancel: &CancelToken,
) -> Result<ProcessOutcome, ProcessError> {
    if spec.sandbox.is_some() {
        return Err(ProcessError::SandboxSetup(crate::SandboxSetupError::new(
            "the strict profile requires Linux namespaces; no Windows sandbox is implemented",
        )));
    }
    validate_platform_strings(spec)?;
    let started = Instant::now();
    let parent_job = parent_job_context()?;
    let mut flags = CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;
    if parent_job.breakaway_allowed {
        flags |= CREATE_BREAKAWAY_FROM_JOB;
    }
    let job = OwnedHandle::new(create_job()?);
    configure_kill_on_close(job.raw())?;
    let mut pipes = stdio_pipes()?;
    let handles = [
        pipes.stdin_child.raw(),
        pipes.stdout_child.raw(),
        pipes.stderr_child.raw(),
    ];
    let mut attribute = AttributeList::new(&handles)?;
    let command_line = build_command_line(&spec.executable, &spec.arguments);
    if command_line.len() > 32_767 {
        return Err(ProcessError::InvalidSpec(
            "the Windows command line exceeds the 32,767 UTF-16 unit limit".to_owned(),
        ));
    }
    let environment = environment_block(&spec.environment)?;
    let application = wide_null(spec.executable.as_os_str());
    let directory = wide_null(spec.working_directory.as_os_str());
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = pipes.stdin_child.raw();
    startup.StartupInfo.hStdOutput = pipes.stdout_child.raw();
    startup.StartupInfo.hStdError = pipes.stderr_child.raw();
    startup.lpAttributeList = attribute.raw();
    let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let mut command_line = command_line;
    // SAFETY: every pointer references a live buffer for the duration of the
    // call. The attribute list restricts inheritance to the three stdio
    // handles, all of which are inheritable.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            flags,
            environment.as_ptr().cast(),
            directory.as_ptr(),
            &startup as *const STARTUPINFOEXW as *const STARTUPINFOW,
            &mut process_info,
        )
    };
    drop(attribute);
    if created == 0 {
        return Err(ProcessError::spawn(
            &spec.executable,
            format!("CreateProcessW failed with {}", last_error()),
        ));
    }
    let process = OwnedHandle::new(process_info.hProcess);
    let thread = OwnedHandle::new(process_info.hThread);
    // The child-side handles are duplicated by CreateProcessW; close ours.
    pipes.close_child_ends();
    // SAFETY: the process and job handles are valid and owned here. The child is
    // still suspended, so assignment is race-free and happens before any worker
    // code can run.
    let assigned = unsafe { AssignProcessToJobObject(job.raw(), process.raw()) };
    if assigned == 0 {
        let code = unsafe { GetLastError() };
        let _ = terminate_job(job.raw());
        // SAFETY: best-effort termination of the suspended child.
        unsafe {
            TerminateProcess(process.raw(), 1);
        }
        wait_bounded(process.raw(), Duration::from_millis(500));
        return Err(ProcessError::Containment(format!(
            "AssignProcessToJobObject failed with error {code}{}",
            if code == ERROR_ACCESS_DENIED && parent_job.in_job {
                " (the parent is already in a job that does not permit nested assignment)"
            } else {
                ""
            }
        )));
    }
    if cancel.is_cancelled() {
        let cleanup = terminate_before_resume(process.raw(), job.raw());
        if !cleanup.direct_reaped {
            return Err(ProcessError::Cleanup(
                "the suspended worker could not be reaped after cancellation".to_owned(),
            ));
        }
        return Ok(cancelled_outcome(
            started,
            process_id(process.raw()),
            cleanup,
        ));
    }
    if Instant::now() >= started + spec.limits.bootstrap_timeout {
        let cleanup = terminate_before_resume(process.raw(), job.raw());
        if !cleanup.direct_reaped {
            return Err(ProcessError::Cleanup(
                "the suspended worker could not be reaped after the bootstrap deadline".to_owned(),
            ));
        }
        return Ok(timed_out_outcome(
            started,
            process_id(process.raw()),
            TimeoutStage::Bootstrap,
            cleanup,
        ));
    }
    // SAFETY: the suspended process thread handle is valid and was returned by
    // CreateProcessW.
    let resumed = unsafe { ResumeThread(thread.raw()) };
    if resumed == u32::MAX {
        let code = unsafe { GetLastError() };
        let _ = terminate_job(job.raw());
        wait_bounded(process.raw(), Duration::from_millis(500));
        return Err(ProcessError::Containment(format!(
            "the suspended worker could not be resumed (error {code})"
        )));
    }
    drop(thread);
    let pid = process_id(process.raw());
    let mut stdout = CapturedOutput::new();
    let mut stderr = CapturedOutput::new();
    let mut writer = stdin_writer(pipes.take_stdin_write(), stdin);
    let end = match supervise_loop(
        process.raw(),
        &mut pipes,
        &mut stdout,
        &mut stderr,
        &mut writer,
        cancel,
        started,
        &spec.limits,
    ) {
        Ok(end) => end,
        Err(error) => {
            let _ = finish(
                process.raw(),
                job.raw(),
                &mut pipes,
                &mut stdout,
                &mut stderr,
                &mut writer,
                &spec.limits,
            );
            return Err(error);
        }
    };
    let cleanup = finish(
        process.raw(),
        job.raw(),
        &mut pipes,
        &mut stdout,
        &mut stderr,
        &mut writer,
        &spec.limits,
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
    let exit = if cleanup.direct_reaped {
        process_exit_code(process.raw()).map(ExitStatus::exited)
    } else {
        None
    };
    let stderr_summary = sanitize_child_stderr(
        stderr.as_bytes(),
        Some(&spec.working_directory.to_string_lossy()),
    );
    Ok(ProcessOutcome {
        end,
        exit,
        stdout,
        stderr,
        stderr_summary,
        elapsed: started.elapsed(),
        process_id: Some(pid),
        cleanup,
    })
}

fn cancelled_outcome(started: Instant, pid: u32, cleanup: TreeCleanup) -> ProcessOutcome {
    ProcessOutcome {
        end: ProcessEnd::Cancelled(CancelStage::Bootstrap),
        exit: None,
        stdout: CapturedOutput::new(),
        stderr: CapturedOutput::new(),
        stderr_summary: String::new(),
        elapsed: started.elapsed(),
        process_id: Some(pid),
        cleanup,
    }
}

fn timed_out_outcome(
    started: Instant,
    pid: u32,
    stage: TimeoutStage,
    cleanup: TreeCleanup,
) -> ProcessOutcome {
    ProcessOutcome {
        end: ProcessEnd::TimedOut(stage),
        exit: None,
        stdout: CapturedOutput::new(),
        stderr: CapturedOutput::new(),
        stderr_summary: String::new(),
        elapsed: started.elapsed(),
        process_id: Some(pid),
        cleanup,
    }
}

fn terminate_before_resume(process: HANDLE, job: HANDLE) -> TreeCleanup {
    let terminated = terminate_job(job);
    let mut direct_reaped = wait_bounded(process, Duration::from_millis(500));
    if !direct_reaped {
        // SAFETY: best-effort hard termination of our own suspended child.
        unsafe {
            TerminateProcess(process, 1);
        }
        direct_reaped = wait_bounded(process, Duration::from_millis(500));
    }
    let tree_empty = active_job_processes(job) == Some(0);
    TreeCleanup {
        direct_reaped,
        tree_terminated: terminated || tree_empty,
        tree_empty,
        pipes_drained: true,
        termination_timed_out: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn supervise_loop(
    process: HANDLE,
    pipes: &mut StdioPipes,
    stdout: &mut CapturedOutput,
    stderr: &mut CapturedOutput,
    writer: &mut Option<StdinWriter>,
    cancel: &CancelToken,
    started: Instant,
    limits: &ProcessLimits,
) -> Result<ProcessEnd, ProcessError> {
    let wall_deadline = started + limits.wall_timeout;
    let bootstrap_deadline = started + limits.bootstrap_timeout;
    let mut stdout_open = true;
    let mut stderr_open = true;
    loop {
        // SAFETY: the process handle is valid.
        let waited = unsafe { WaitForSingleObject(process, 0) };
        if waited == WAIT_OBJECT_0 {
            let status = process_exit_code(process).ok_or_else(|| {
                ProcessError::Wait("the exited process did not expose a final exit code".to_owned())
            })?;
            return Ok(ProcessEnd::Exited(ExitStatus::exited(status)));
        }
        if waited == WAIT_FAILED {
            return Err(ProcessError::Wait(format!(
                "WaitForSingleObject failed with {}",
                last_error()
            )));
        }
        if waited != WAIT_TIMEOUT {
            return Err(ProcessError::Wait(format!(
                "WaitForSingleObject returned {waited}"
            )));
        }
        if stdout_open {
            let handle = pipes
                .stdout_read
                .as_ref()
                .expect("stdout pipe is open")
                .raw();
            match read_pipe(handle, stdout, limits.max_stdout_bytes) {
                Ok(ReadState::Eof) => {
                    stdout_open = false;
                    pipes.close_stdout_read();
                }
                Ok(ReadState::Data | ReadState::WouldBlock) => {}
                Err(error) => {
                    return Err(ProcessError::Io(format!("stdout read failed: {error}")));
                }
            }
            if stdout.truncated() && stdout.len() >= limits.max_stdout_bytes {
                return Ok(ProcessEnd::OutputLimitExceeded(OutputStream::Stdout));
            }
        }
        if stderr_open {
            let handle = pipes
                .stderr_read
                .as_ref()
                .expect("stderr pipe is open")
                .raw();
            match read_pipe(handle, stderr, limits.max_stderr_bytes) {
                Ok(ReadState::Eof) => {
                    stderr_open = false;
                    pipes.close_stderr_read();
                }
                Ok(ReadState::Data | ReadState::WouldBlock) => {}
                Err(error) => {
                    return Err(ProcessError::Io(format!("stderr read failed: {error}")));
                }
            }
        }
        if cancel.is_cancelled() {
            return Ok(ProcessEnd::Cancelled(CancelStage::Running));
        }
        let now = Instant::now();
        let stdin_pending = writer.as_ref().is_some_and(|writer| !writer.finished());
        if stdin_pending && now >= bootstrap_deadline && bootstrap_deadline <= wall_deadline {
            return Ok(ProcessEnd::TimedOut(TimeoutStage::Bootstrap));
        }
        if now >= wall_deadline {
            return Ok(ProcessEnd::TimedOut(TimeoutStage::WallClock));
        }
        if let Some(active) = writer.as_ref() {
            if active.finished() {
                let active = writer.take().expect("writer checked above");
                active.join();
            }
        }
        std::thread::sleep(Duration::from_millis(POLL_MS));
    }
}

#[allow(clippy::too_many_arguments)]
fn finish(
    process: HANDLE,
    job: HANDLE,
    pipes: &mut StdioPipes,
    stdout: &mut CapturedOutput,
    stderr: &mut CapturedOutput,
    writer: &mut Option<StdinWriter>,
    limits: &ProcessLimits,
) -> TreeCleanup {
    let deadline = Instant::now() + limits.termination_timeout;
    let terminated = terminate_job(job);
    if let Some(writer) = writer.take() {
        if writer.finished() {
            writer.join();
        }
    }
    let mut stdout_open = pipes.stdout_read.is_some();
    let mut stderr_open = pipes.stderr_read.is_some();
    while Instant::now() < deadline {
        if stdout_open {
            if let Some(handle) = pipes.stdout_read.as_ref() {
                match read_pipe(handle.raw(), stdout, limits.max_stdout_bytes) {
                    Ok(ReadState::Eof) => {
                        pipes.close_stdout_read();
                        stdout_open = false;
                    }
                    Ok(ReadState::Data | ReadState::WouldBlock) => {}
                    Err(_) => {
                        pipes.close_stdout_read();
                        stdout_open = false;
                    }
                }
            } else {
                stdout_open = false;
            }
        }
        if stderr_open {
            if let Some(handle) = pipes.stderr_read.as_ref() {
                match read_pipe(handle.raw(), stderr, limits.max_stderr_bytes) {
                    Ok(ReadState::Eof) => {
                        pipes.close_stderr_read();
                        stderr_open = false;
                    }
                    Ok(ReadState::Data | ReadState::WouldBlock) => {}
                    Err(_) => {
                        pipes.close_stderr_read();
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
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut direct_reaped =
        wait_bounded(process, deadline.saturating_duration_since(Instant::now()));
    if !direct_reaped {
        // SAFETY: best-effort hard termination of our own child.
        unsafe {
            TerminateProcess(process, 1);
        }
        direct_reaped = wait_bounded(process, Duration::from_millis(500));
    }
    let mut tree_empty = false;
    while Instant::now() < deadline {
        match active_job_processes(job) {
            Some(0) => {
                tree_empty = true;
                break;
            }
            Some(_) => {}
            None => {
                tree_empty = false;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    TreeCleanup {
        direct_reaped,
        tree_terminated: terminated || active_job_processes(job) == Some(0),
        tree_empty,
        pipes_drained: !stdout_open && !stderr_open,
        termination_timed_out: Instant::now() >= deadline,
    }
}

fn wait_bounded(process: HANDLE, timeout: Duration) -> bool {
    let millis = timeout.as_millis().min(u32::MAX as u128) as u32;
    // SAFETY: the process handle is valid.
    let result = unsafe { WaitForSingleObject(process, millis) };
    result == WAIT_OBJECT_0
}

fn process_exit_code(process: HANDLE) -> Option<i64> {
    let mut code: u32 = 0;
    // SAFETY: the process handle is valid.
    if unsafe { GetExitCodeProcess(process, &mut code) } == 0 {
        return None;
    }
    Some(i64::from(code))
}

fn process_id(process: HANDLE) -> u32 {
    #[allow(unused_imports)]
    use windows_sys::Win32::System::Threading::GetProcessId;
    // SAFETY: the process handle is valid.
    unsafe { GetProcessId(process) }
}

struct ParentJobContext {
    in_job: bool,
    breakaway_allowed: bool,
}

fn parent_job_context() -> Result<ParentJobContext, ProcessError> {
    let mut in_job: i32 = 0;
    // SAFETY: NULL means "the current process's job".
    let queried = unsafe { IsProcessInJob(GetCurrentProcess(), ptr::null_mut(), &mut in_job) };
    if queried == 0 {
        return Err(ProcessError::Containment(format!(
            "IsProcessInJob failed with {}",
            last_error()
        )));
    }
    if in_job == 0 {
        return Ok(ParentJobContext {
            in_job: false,
            breakaway_allowed: false,
        });
    }
    let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
    // SAFETY: the information buffer is correctly sized and the NULL job
    // handle queries the current process's job.
    let result = unsafe {
        QueryInformationJobObject(
            ptr::null_mut(),
            JobObjectExtendedLimitInformation,
            (&mut information as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size,
            ptr::null_mut(),
        )
    };
    let flags = if result != 0 {
        information.BasicLimitInformation.LimitFlags
    } else {
        0
    };
    // SILENT_BREAKAWAY_OK already prevents the child from joining the parent
    // job; CREATE_BREAKAWAY_FROM_JOB is only valid with BREAKAWAY_OK.
    let breakaway_allowed = flags & JOB_OBJECT_LIMIT_BREAKAWAY_OK != 0;
    let _silent = flags & JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK != 0;
    Ok(ParentJobContext {
        in_job: true,
        breakaway_allowed,
    })
}

fn create_job() -> Result<HANDLE, ProcessError> {
    // SAFETY: no security attributes or name are required.
    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job.is_null() {
        return Err(ProcessError::Containment(format!(
            "CreateJobObjectW failed with {}",
            last_error()
        )));
    }
    Ok(job)
}

fn configure_kill_on_close(job: HANDLE) -> Result<(), ProcessError> {
    let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
    // SAFETY: the information buffer is correctly sized for the class.
    let result = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&information as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size,
        )
    };
    if result == 0 {
        return Err(ProcessError::Containment(format!(
            "SetInformationJobObject(kill-on-close) failed with {}",
            last_error()
        )));
    }
    Ok(())
}

fn active_job_processes(job: HANDLE) -> Option<u32> {
    let mut information: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32;
    // SAFETY: the information buffer is correctly sized for the class.
    let result = unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicAccountingInformation,
            (&mut information as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            size,
            ptr::null_mut(),
        )
    };
    (result != 0).then_some(information.ActiveProcesses)
}

fn terminate_job(job: HANDLE) -> bool {
    // SAFETY: best-effort termination of the job we own.
    unsafe { TerminateJobObject(job, 1) != 0 }
}

struct AttributeList {
    storage: Vec<usize>,
}

impl AttributeList {
    fn new(handles: &[HANDLE]) -> Result<Self, ProcessError> {
        let mut size = 0usize;
        // SAFETY: a zero-sized probe is documented to set the required size.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
        }
        if size == 0 {
            return Err(ProcessError::Containment(format!(
                "InitializeProcThreadAttributeList sizing failed with {}",
                last_error()
            )));
        }
        let width = std::mem::size_of::<usize>();
        let mut storage = vec![0usize; size.div_ceil(width)];
        let list = storage.as_mut_ptr().cast::<c_void>();
        // SAFETY: the storage is aligned and at least `size` bytes.
        let initialized = unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) };
        if initialized == 0 {
            return Err(ProcessError::Containment(format!(
                "InitializeProcThreadAttributeList failed with {}",
                last_error()
            )));
        }
        // SAFETY: the attribute list is initialised and the handle array lives
        // until CreateProcessW returns (it is borrowed by the caller).
        let updated = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0 {
            // SAFETY: the list was initialised above.
            unsafe {
                DeleteProcThreadAttributeList(list);
            }
            return Err(ProcessError::Containment(format!(
                "UpdateProcThreadAttribute(handle list) failed with {}",
                last_error()
            )));
        }
        Ok(Self { storage })
    }

    fn raw(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast()
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: the list was initialised by `new`.
        unsafe {
            DeleteProcThreadAttributeList(self.storage.as_mut_ptr().cast());
        }
    }
}

struct OwnedHandle {
    handle: HANDLE,
}

// SAFETY: `OwnedHandle` has unique ownership of one kernel handle. Moving that
// ownership to another thread is safe; it is never duplicated or aliased.
unsafe impl Send for OwnedHandle {}

impl OwnedHandle {
    fn new(handle: HANDLE) -> Self {
        Self { handle }
    }

    fn raw(&self) -> HANDLE {
        self.handle
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: the handle is owned by this wrapper.
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

struct StdioPipes {
    stdin_child: OwnedHandle,
    stdin_write: Option<OwnedHandle>,
    stdout_child: OwnedHandle,
    stdout_read: Option<OwnedHandle>,
    stderr_child: OwnedHandle,
    stderr_read: Option<OwnedHandle>,
}

impl StdioPipes {
    fn take_stdin_write(&mut self) -> Option<OwnedHandle> {
        self.stdin_write.take()
    }

    fn close_stdout_read(&mut self) {
        self.stdout_read = None;
    }

    fn close_stderr_read(&mut self) {
        self.stderr_read = None;
    }

    fn close_child_ends(&mut self) {
        // Owned handles are closed by replacing them with impossible sentinels;
        // use a dedicated close method instead.
        self.stdin_child.close_now();
        self.stdout_child.close_now();
        self.stderr_child.close_now();
    }
}

impl OwnedHandle {
    fn close_now(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: owned handle, closed once.
            unsafe {
                CloseHandle(self.handle);
            }
            self.handle = ptr::null_mut();
        }
    }
}

fn stdio_pipes() -> Result<StdioPipes, ProcessError> {
    // Direction contract:
    // * stdin:  parent writes, child reads  -> create_pipe(true)  = (parent, child)
    // * stdout: child writes, parent reads  -> create_pipe(false) = (parent, child)
    // * stderr: same as stdout.
    let (stdin_write, stdin_child) = create_pipe(true)?;
    let (stdout_read, stdout_child) = create_pipe(false)?;
    let (stderr_read, stderr_child) = create_pipe(false)?;
    Ok(StdioPipes {
        stdin_child,
        stdin_write: Some(stdin_write),
        stdout_child,
        stdout_read: Some(stdout_read),
        stderr_child,
        stderr_read: Some(stderr_read),
    })
}

/// Create an anonymous pipe and return `(parent_end, child_end)`.
///
/// `child_reads` selects the child's direction: `true` gives the child the read
/// end (stdin), `false` gives it the write end (stdout/stderr). The child end is
/// inheritable; the parent end is explicitly made non-inheritable.
fn create_pipe(child_reads: bool) -> Result<(OwnedHandle, OwnedHandle), ProcessError> {
    // Create both ends non-inheritable, then mark only the child end
    // inheritable. This means a parent handle is never briefly inheritable by
    // an unrelated concurrent CreateProcessW call.
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 0,
    };
    let mut read: HANDLE = ptr::null_mut();
    let mut write: HANDLE = ptr::null_mut();
    // SAFETY: both output handles are valid locations and the attributes are
    // live for the call.
    let result = unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) };
    if result == 0 {
        return Err(ProcessError::Containment(format!(
            "CreatePipe failed with {}",
            last_error()
        )));
    }
    let (parent, child) = if child_reads {
        (write, read)
    } else {
        (read, write)
    };
    let parent = OwnedHandle::new(parent);
    let child = OwnedHandle::new(child);
    // SAFETY: both handles are valid and owned by the wrappers on every return
    // path below.
    if unsafe { SetHandleInformation(parent.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(ProcessError::Containment(format!(
            "SetHandleInformation(parent, no-inherit) failed with error {}",
            last_error()
        )));
    }
    // Only the child end becomes inheritable, and only after the parent end has
    // been explicitly cleared. The direction/inheritance contract is explicit.
    if unsafe { SetHandleInformation(child.raw(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        return Err(ProcessError::Containment(format!(
            "SetHandleInformation(child, inherit) failed with error {}",
            last_error()
        )));
    }
    Ok((parent, child))
}

enum ReadState {
    Data,
    WouldBlock,
    Eof,
}

fn read_pipe(handle: HANDLE, capture: &mut CapturedOutput, limit: usize) -> io::Result<ReadState> {
    let mut available: u32 = 0;
    // SAFETY: the handle is a pipe read end; all pointers are valid.
    let peeked = unsafe {
        PeekNamedPipe(
            handle,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            &mut available,
            ptr::null_mut(),
        )
    };
    if peeked == 0 {
        let code = unsafe { GetLastError() };
        return if code == ERROR_BROKEN_PIPE || code == ERROR_PIPE_NOT_CONNECTED {
            Ok(ReadState::Eof)
        } else {
            Err(io::Error::from_raw_os_error(code as i32))
        };
    }
    if available == 0 {
        return Ok(ReadState::WouldBlock);
    }
    let mut buffer = [0u8; READ_CHUNK];
    let mut read: u32 = 0;
    // SAFETY: the buffer is valid and the handle is a pipe read end.
    let result = unsafe {
        ReadFile(
            handle,
            buffer.as_mut_ptr(),
            (available as usize).min(buffer.len()) as u32,
            &mut read,
            ptr::null_mut(),
        )
    };
    if result == 0 {
        let code = unsafe { GetLastError() };
        return if code == ERROR_BROKEN_PIPE || code == ERROR_PIPE_NOT_CONNECTED {
            Ok(ReadState::Eof)
        } else {
            Err(io::Error::from_raw_os_error(code as i32))
        };
    }
    if read == 0 {
        return Ok(ReadState::Eof);
    }
    capture.push(&buffer[..read as usize], limit);
    Ok(ReadState::Data)
}

struct SendHandle(OwnedHandle);
// SAFETY: the wrapper transfers ownership of one kernel handle to exactly one
// writer thread.
unsafe impl Send for SendHandle {}

struct StdinWriter {
    handle: std::thread::JoinHandle<()>,
}

impl StdinWriter {
    fn finished(&self) -> bool {
        self.handle.is_finished()
    }

    fn join(self) {
        let _ = self.handle.join();
    }
}

fn stdin_writer(handle: Option<OwnedHandle>, payload: &[u8]) -> Option<StdinWriter> {
    let handle = handle?;
    if payload.is_empty() {
        return None;
    }
    let payload = payload.to_vec();
    let handle = SendHandle(handle);
    Some(StdinWriter {
        handle: std::thread::spawn(move || {
            let SendHandle(owned) = handle;
            let mut offset = 0usize;
            while offset < payload.len() {
                let mut written: u32 = 0;
                // SAFETY: the pipe write handle is owned by this thread and the
                // payload slice is valid.
                let result = unsafe {
                    WriteFile(
                        owned.raw(),
                        payload[offset..].as_ptr(),
                        (payload.len() - offset).min(u32::MAX as usize) as u32,
                        &mut written,
                        ptr::null_mut(),
                    )
                };
                if result == 0 {
                    break;
                }
                if written == 0 {
                    break;
                }
                offset += written as usize;
            }
            drop(owned);
        }),
    })
}

fn build_command_line(executable: &std::path::Path, arguments: &[OsString]) -> Vec<u16> {
    let mut output = Vec::new();
    append_argument(&mut output, executable.as_os_str());
    for argument in arguments {
        output.push(b' ' as u16);
        append_argument(&mut output, argument);
    }
    output.push(0);
    output
}

fn append_argument(output: &mut Vec<u16>, argument: &OsStr) {
    let encoded: Vec<u16> = argument.encode_wide().collect();
    let needs_quotes = encoded.is_empty()
        || encoded
            .iter()
            .any(|unit| *unit == b' ' as u16 || *unit == b'\t' as u16 || *unit == b'"' as u16);
    if !needs_quotes {
        output.extend_from_slice(&encoded);
        return;
    }
    output.push(b'"' as u16);
    let mut backslashes = 0usize;
    for unit in encoded {
        if unit == b'\\' as u16 {
            backslashes += 1;
            continue;
        }
        if unit == b'"' as u16 {
            for _ in 0..(backslashes * 2 + 1) {
                output.push(b'\\' as u16);
            }
            output.push(b'"' as u16);
            backslashes = 0;
            continue;
        }
        for _ in 0..backslashes {
            output.push(b'\\' as u16);
        }
        backslashes = 0;
        output.push(unit);
    }
    for _ in 0..(backslashes * 2) {
        output.push(b'\\' as u16);
    }
    output.push(b'"' as u16);
}

fn validate_platform_strings(spec: &ProcessSpec) -> Result<(), ProcessError> {
    validate_wide(spec.executable.as_os_str(), "executable")?;
    for argument in &spec.arguments {
        validate_wide(argument.as_os_str(), "argument")?;
    }
    validate_wide(spec.working_directory.as_os_str(), "working directory")?;
    spec.environment.validate()?;
    Ok(())
}

fn validate_wide(value: &OsStr, label: &str) -> Result<(), ProcessError> {
    if value.encode_wide().any(|unit| unit == 0) {
        return Err(ProcessError::InvalidSpec(format!(
            "the {label} contains a null byte"
        )));
    }
    Ok(())
}

fn environment_block(environment: &ChildEnvironment) -> Result<Vec<u16>, ProcessError> {
    // Reject any entry that escaped aggregate validation before it can inject a
    // NUL-separated variable into the Windows environment block.
    environment.validate()?;
    let mut block = Vec::new();
    for entry in environment.entries() {
        let variable = format!("{}={}", entry.key, entry.value);
        block.extend(variable.encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn last_error() -> u32 {
    // SAFETY: GetLastError has no preconditions.
    unsafe { GetLastError() }
}

/// Report whether the current process is already in a Windows Job Object.
///
/// Returns `None` when `IsProcessInJob` itself fails.
pub fn current_process_in_job() -> Option<bool> {
    let mut in_job: i32 = 0;
    // SAFETY: NULL queries the current process.
    let queried = unsafe { IsProcessInJob(GetCurrentProcess(), ptr::null_mut(), &mut in_job) };
    (queried != 0).then_some(in_job != 0)
}

/// The raw Win32 handle count for the current process, used by tests and
/// leak diagnostics.
pub fn current_handle_count() -> Option<u32> {
    let mut count = 0u32;
    // SAFETY: GetCurrentProcess is a pseudo-handle and the count pointer is
    // valid.
    let result = unsafe {
        windows_sys::Win32::System::Threading::GetProcessHandleCount(
            GetCurrentProcess(),
            &mut count,
        )
    };
    (result != 0).then_some(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::GetHandleInformation;

    fn inheritance(handle: HANDLE) -> u32 {
        let mut flags = 0u32;
        // SAFETY: the handle is owned by the caller and the flag pointer is
        // valid.
        let result = unsafe { GetHandleInformation(handle, &mut flags) };
        assert_ne!(result, 0, "GetHandleInformation failed");
        flags & HANDLE_FLAG_INHERIT
    }

    fn write_handle(handle: HANDLE, payload: &[u8]) {
        let mut written = 0u32;
        // SAFETY: the handle is a pipe write end owned by the caller.
        let result = unsafe {
            WriteFile(
                handle,
                payload.as_ptr(),
                payload.len() as u32,
                &mut written,
                ptr::null_mut(),
            )
        };
        assert_ne!(result, 0, "WriteFile failed");
        assert_eq!(written as usize, payload.len());
    }

    fn read_to_eof(handle: HANDLE, limit: usize) -> CapturedOutput {
        let mut capture = CapturedOutput::new();
        for _ in 0..10_000 {
            match read_pipe(handle, &mut capture, limit).expect("pipe read") {
                ReadState::Eof => return capture,
                ReadState::Data => {}
                ReadState::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        panic!("pipe did not reach EOF");
    }

    #[test]
    #[ignore = "requires a Windows host; runtime proof is pending"]
    fn stdio_pipe_direction_inheritance_and_round_trip_are_correct() {
        let mut pipes = stdio_pipes().expect("stdio pipes");

        // stdin: the child must receive the read end; the parent writes.
        assert_eq!(inheritance(pipes.stdin_child.raw()), HANDLE_FLAG_INHERIT);
        assert_eq!(
            inheritance(pipes.stdin_write.as_ref().expect("stdin write").raw()),
            0
        );
        // stdout/stderr: the child must receive the write end; the parent reads.
        assert_eq!(inheritance(pipes.stdout_child.raw()), HANDLE_FLAG_INHERIT);
        assert_eq!(
            inheritance(pipes.stdout_read.as_ref().expect("stdout read").raw()),
            0
        );
        assert_eq!(inheritance(pipes.stderr_child.raw()), HANDLE_FLAG_INHERIT);
        assert_eq!(
            inheritance(pipes.stderr_read.as_ref().expect("stderr read").raw()),
            0
        );

        let stdout_payload = b"stdout round trip";
        write_handle(pipes.stdout_child.raw(), stdout_payload);
        pipes.stdout_child.close_now();
        let stdout = read_to_eof(
            pipes.stdout_read.as_ref().expect("stdout read").raw(),
            1_024,
        );
        assert_eq!(stdout.as_bytes(), stdout_payload);

        let stderr_payload = b"stderr round trip";
        write_handle(pipes.stderr_child.raw(), stderr_payload);
        pipes.stderr_child.close_now();
        let stderr = read_to_eof(
            pipes.stderr_read.as_ref().expect("stderr read").raw(),
            1_024,
        );
        assert_eq!(stderr.as_bytes(), stderr_payload);

        // stdin round trip: write through the parent end and read the child end.
        let stdin_payload = b"stdin round trip";
        let mut stdin_write = pipes.take_stdin_write().expect("stdin write");
        write_handle(stdin_write.raw(), stdin_payload);
        stdin_write.close_now();
        let stdin = read_to_eof(pipes.stdin_child.raw(), 1_024);
        assert_eq!(stdin.as_bytes(), stdin_payload);
    }

    #[test]
    #[ignore = "requires a Windows host; runtime proof is pending"]
    fn interior_nul_argv_is_refused_before_create_process() {
        let executable = std::env::current_exe().expect("current executable");
        let directory = executable.parent().expect("executable directory");
        let mut spec = ProcessSpec::new(&executable, directory).expect("spec");
        spec = spec.arg("bad\0argument");
        // The platform validator runs before job or process creation.
        assert!(matches!(
            validate_platform_strings(&spec),
            Err(ProcessError::InvalidSpec(_))
        ));
    }
}
