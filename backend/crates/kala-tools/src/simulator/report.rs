//! The reference report.
//!
//! The simulator can publish a JSON report of one run inside its workspace.
//! The report is written atomically and without following a planted symlink:
//! the parent directory is created and re-checked with a single `lstat`, the
//! payload is written to a fresh exclusive file and renamed into place, so a
//! reader never observes a partial artifact and a concurrent writer cannot
//! redirect the report.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use kala_contracts::{ArtifactClassification, ArtifactRef, ContractModel};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};

use crate::protocol::{MAX_JSON_BYTES, canonical_json_dumps, resolve_workspace_path, sha256_hex};
use crate::simulator::error::SimulatorError;
use crate::simulator::parameters::{SimulatorParameters, report_payload};

/// Publish the reference report of one run.
///
/// The report is written to `outputs/reference-report.json` inside the
/// workspace. The failure is a `SimulatorInputError`.
pub fn write_report(
    workspace: &Path,
    parameters: &SimulatorParameters,
    metrics: &CanonicalValue,
    call_id: &str,
) -> Result<(ArtifactRef, String), SimulatorError> {
    let relative = format!(
        "{}/{}",
        crate::simulator::SIMULATOR_REPORT_DIRECTORY,
        crate::simulator::SIMULATOR_REPORT_NAME
    );
    let target = resolve_workspace_path(workspace, &relative)
        .map_err(|_| SimulatorError::input("the report path is not inside the workspace"))?;
    let payload = report_payload(parameters, metrics, call_id);
    let encoded = canonical_json_dumps(&payload, MAX_JSON_BYTES)
        .map_err(|error| SimulatorError::Json(error.to_string()))?;
    let mut bytes = encoded.into_bytes();
    bytes.push(b'\n');
    write_workspace_file(&target, &bytes)?;
    let artifact = ArtifactRef::from_value(&CanonicalValue::Object({
        let mut entries = CanonicalObject::new();
        entries.insert(
            JsonText::new("artifact_id"),
            CanonicalValue::text(format!("{call_id}-report")),
        );
        entries.insert(
            JsonText::new("uri"),
            CanonicalValue::text(format!("file://{}", resolved_posix(&target))),
        );
        entries.insert(
            JsonText::new("digest"),
            CanonicalValue::text(sha256_hex(&bytes)),
        );
        entries.insert(
            JsonText::new("media_type"),
            CanonicalValue::text("application/json"),
        );
        entries.insert(
            JsonText::new("size_bytes"),
            CanonicalValue::integer(bytes.len() as i64),
        );
        entries.insert(
            JsonText::new("classification"),
            CanonicalValue::text(ArtifactClassification::Text.as_str()),
        );
        entries
    }))
    .map_err(|error| SimulatorError::contract(&error))?;
    Ok((artifact, relative))
}

/// Publish a file inside the workspace, atomically and without a symlink.
///
/// The parent directory is created with a private mode and re-checked with a
/// single `lstat`, so a symlinked directory is refused. The payload is
/// written to a fresh exclusive file and renamed into place, so a reader
/// never observes a partial artifact and a failed publication leaves no
/// temporary file behind.
pub fn write_workspace_file(target: &Path, payload: &[u8]) -> Result<(), SimulatorError> {
    let Some(parent) = target.parent() else {
        return Err(SimulatorError::input(
            "the report directory could not be created: No such file or directory",
        ));
    };
    make_directories(parent).map_err(|error| {
        SimulatorError::input(format!(
            "the report directory could not be created: {}",
            python_strerror(&error)
        ))
    })?;
    let metadata = fs::symlink_metadata(parent)
        .map_err(|_| SimulatorError::input("the report directory is not accessible"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SimulatorError::input(
            "the report directory must be a real directory",
        ));
    }
    let temporary = parent.join(format!(
        ".{}.{}.{}.partial",
        target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id(),
        thread_identifier()
    ));
    let write = || -> std::io::Result<()> {
        let mut file = open_exclusive(&temporary)?;
        file.write_all(payload)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, target)
    };
    if let Err(error) = write() {
        let _ = fs::remove_file(&temporary);
        return Err(SimulatorError::input(format!(
            "the report could not be written: {}",
            python_strerror(&error)
        )));
    }
    Ok(())
}

/// Open a file that must not exist yet.
///
/// The exclusive create refuses an existing path, including a symbolic link,
/// so the report can never be written through a link that was planted before
/// the call.
fn open_exclusive(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Create a directory and its missing ancestors.
///
/// The ancestors get the default mode and the leaf gets the private mode, so
/// the report directory is not readable by another user.
fn make_directories(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    match create_directory(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if path.is_dir() {
                Ok(())
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

/// Create one directory with a private mode.
fn create_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut builder = fs::DirBuilder::new();
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
        builder.create(path)
    }
    #[cfg(not(unix))]
    {
        // The host does not have POSIX modes, so the privacy of the report
        // directory is the responsibility of the sandbox profile.
        fs::DirBuilder::new().create(path)
    }
}

/// The resolved, POSIX form of a path.
///
/// The Python implementation resolves the target with
/// `Path.resolve(strict=False)` and formats it with `as_posix()`.
fn resolved_posix(target: &Path) -> String {
    let resolved = match target.parent() {
        Some(parent) => match parent.canonicalize() {
            Ok(parent) => parent.join(target.file_name().unwrap_or_default()),
            Err(_) => target.to_path_buf(),
        },
        None => target.to_path_buf(),
    };
    resolved.to_string_lossy().replace('\\', "/")
}

/// The `strerror` text of an operating system failure.
/// The `strerror` text of an operating system failure.
///
/// CPython reports `error.strerror`, which is the message of the C library
/// without the error number. The Rust message carries the same text with the
/// error number in a suffix, which is removed here.
fn python_strerror(error: &std::io::Error) -> String {
    let text = error.to_string();
    match error.raw_os_error() {
        Some(code) => {
            let suffix = format!(" (os error {code})");
            text.strip_suffix(&suffix)
                .unwrap_or(text.as_str())
                .to_owned()
        }
        None => text,
    }
}

/// A stable identifier of the calling thread.
///
/// The Python implementation uses `threading.get_ident()` in the name of the
/// temporary file. The Rust standard library does not expose a thread
/// identifier, so every thread is given one from a process wide counter.
fn thread_identifier() -> u64 {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};
    thread_local! {
        static IDENTIFIER: Cell<u64> = const { Cell::new(0) };
    }
    IDENTIFIER.with(|identifier| {
        if identifier.get() == 0 {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            identifier.set(NEXT.fetch_add(1, Ordering::Relaxed));
        }
        identifier.get()
    })
}
