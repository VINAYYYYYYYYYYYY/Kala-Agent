//! Run directory and workspace paths.
//!
//! The executor owns the run directory. Every call gets a workspace below it,
//! the workspace is created with a single `lstat` check so a symlink that was
//! planted in between is refused, and a call identifier can never address a
//! directory component outside the run directory.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use kala_sandbox::resolve_strict_false;

use crate::executor::error::{ExecutorResult, ToolExecutorError};

/// The message of a refusal that the run directory path is a file.
const PATH_EXISTS_AS_FILE: &str = "the run directory path exists as a file";

/// The message of a refusal that the run directory path is not a real
/// directory.
const PATH_MUST_BE_A_DIRECTORY: &str = "run directory paths must be real directories";

/// The `strerror` of an operating system error.
///
/// CPython reports the bare message of the error, Rust appends the numeric
/// code in parentheses. The executor reports the bare message, so a refusal
/// explains itself the same way in both implementations.
pub fn strerror(error: &io::Error) -> String {
    let text = error.to_string();
    match text.find(" (os error ") {
        Some(index) => text[..index].to_owned(),
        None => text,
    }
}

/// Create a directory that concurrent callers may be creating as well.
///
/// The check and the creation cannot be separated, so the directory type is
/// re-read with a single `lstat` afterwards and anything that is not a real
/// directory, including a symlink planted in between, is refused.
pub fn ensure_directory(path: &Path) -> ExecutorResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let is_symlink = metadata.file_type().is_symlink();
            if !is_symlink && metadata.is_dir() {
                return Ok(());
            }
            if is_symlink && path.is_dir() {
                return Err(ToolExecutorError::workspace(PATH_MUST_BE_A_DIRECTORY));
            }
            return Err(ToolExecutorError::workspace(PATH_EXISTS_AS_FILE));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ToolExecutorError::workspace(format!(
                "the run directory path could not be created: {}",
                strerror(&error)
            )));
        }
    }
    match fs::create_dir_all(path) {
        Ok(()) => {
            restrict_mode(path);
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            Err(ToolExecutorError::workspace(PATH_EXISTS_AS_FILE))
        }
        Err(error) => Err(ToolExecutorError::workspace(format!(
            "the run directory path could not be created: {}",
            strerror(&error)
        ))),
    }
}

/// Restrict a directory to the owner, like `os.makedirs(mode=0o700)`.
fn restrict_mode(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.is_dir() {
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Prepare the run directory of an executor.
///
/// The run directory may not be a symlink, may not be a filesystem root and
/// must be a real directory.
pub fn prepare_run_directory(path: &Path) -> ExecutorResult<PathBuf> {
    if path.is_symlink() {
        return Err(ToolExecutorError::workspace(
            "the run directory cannot be a symlink",
        ));
    }
    let resolved = resolve_strict_false(path);
    if resolved.parent().is_none() {
        return Err(ToolExecutorError::workspace(
            "the run directory cannot be a filesystem root",
        ));
    }
    ensure_directory(&resolved)?;
    if resolved.is_symlink() || !resolved.is_dir() {
        return Err(ToolExecutorError::workspace(
            "the run directory must be a directory",
        ));
    }
    Ok(resolved)
}

/// The run directory component of a call identifier.
///
/// A call identifier addresses exactly one directory below the run directory.
/// It may not be empty, it may not be longer than 128 code points, it may not
/// be `.` or `..`, it may not contain a separator, a null byte, a whitespace
/// character or a control character.
pub fn call_component(call_id: &str) -> ExecutorResult<String> {
    let normalized = call_id.trim_matches(is_python_whitespace);
    if normalized.is_empty() || normalized.chars().count() > 128 {
        return Err(ToolExecutorError::workspace(
            "call_id must be nonempty and bounded",
        ));
    }
    if normalized == "." || normalized == ".." {
        return Err(ToolExecutorError::workspace(
            "call_id cannot address a run directory component",
        ));
    }
    if normalized.contains('/')
        || normalized.contains('\\')
        || normalized.contains('\u{0}')
        || normalized
            .chars()
            .any(|character| is_python_whitespace(character) || (character as u32) < 32)
    {
        return Err(ToolExecutorError::workspace(
            "call_id cannot address a run directory component",
        ));
    }
    Ok(normalized.to_owned())
}

/// Whether a character is whitespace for CPython's `str.strip()` and
/// `str.isspace()`.
fn is_python_whitespace(character: char) -> bool {
    matches!(
        u32::from(character),
        0x09..=0x0D
            | 0x1C..=0x1F
            | 0x20
            | 0x85
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

/// The workspace of a call, without creating anything on disk.
pub fn workspace_path(run_directory: &Path, call_id: &str) -> ExecutorResult<PathBuf> {
    let component = call_component(call_id)?;
    Ok(resolve_strict_false(
        &run_directory
            .join("calls")
            .join(component)
            .join("workspace"),
    ))
}

/// The workspace of a call, created and confined to the run directory.
pub fn workspace_for(run_directory: &Path, call_id: &str) -> ExecutorResult<PathBuf> {
    let workspace = workspace_path(run_directory, call_id)?;
    ensure_directory(&workspace)?;
    if !workspace.starts_with(run_directory) {
        return Err(ToolExecutorError::workspace(
            "workspace escapes the run directory",
        ));
    }
    Ok(workspace)
}

/// Normalise a path the way `os.path.normpath` does.
///
/// The result is absolute, has no `.` component and no repeated separator,
/// and every `..` component is folded away.
pub fn normpath(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text.is_empty() {
        return PathBuf::from(".");
    }
    let absolute = text.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if let Some(last) = parts.last() {
                    if *last != ".." {
                        parts.pop();
                        continue;
                    }
                }
                if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let mut output = String::new();
    if absolute {
        output.push('/');
    }
    output.push_str(&parts.join("/"));
    if output.is_empty() {
        output.push('.');
    }
    PathBuf::from(output)
}

/// The components of a path, the way `PurePath.parts` reports them.
pub fn parts(path: &Path) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => result.push("/".to_owned()),
            Component::Prefix(prefix) => {
                result.push(prefix.as_os_str().to_string_lossy().into_owned())
            }
            Component::CurDir => {}
            Component::ParentDir => result.push("..".to_owned()),
            Component::Normal(name) => result.push(name.to_string_lossy().into_owned()),
        }
    }
    result
}
