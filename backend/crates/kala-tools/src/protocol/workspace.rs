//! Workspace path resolution.
//!
//! A tool may only touch files inside its run directory. The Python module
//! resolves a relative path against the run directory, refuses any traversal
//! that leaves it and refuses any symlink on the way, because a symlink is a
//! way out of the sandbox that does not look like one.

use std::path::{Path, PathBuf};

use crate::protocol::MAX_PATH_LENGTH;
use crate::protocol::MAX_PATH_SEGMENTS;
use crate::protocol::error::ToolProtocolError;

/// Resolve a path the way `pathlib.Path.resolve(strict=False)` does.
///
/// The path is made absolute, the longest existing prefix is resolved with
/// the operating system and the remaining components are appended. A `..`
/// component is folded away, as CPython folds it.
pub(crate) fn resolve_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(current) => current.join(path),
            Err(_) => path.to_path_buf(),
        }
    };
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    let mut current = absolute.clone();
    loop {
        if let Ok(resolved) = current.canonicalize() {
            let mut result = resolved;
            for component in suffix.iter().rev() {
                if component == ".." {
                    result.pop();
                } else {
                    result.push(component);
                }
            }
            return result;
        }
        let Some(name) = current.file_name() else {
            return absolute;
        };
        suffix.push(name.to_os_string());
        let Some(parent) = current.parent() else {
            return absolute;
        };
        if parent.as_os_str().is_empty() {
            return absolute;
        }
        current = parent.to_path_buf();
    }
}

/// The POSIX components of a relative path, as `PurePosixPath.parts` sees
/// them.
fn posix_parts(relative: &str) -> Vec<String> {
    relative
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(|part| part.to_owned())
        .collect()
}

/// Whether a segment matches `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`.
fn is_safe_segment(segment: &str) -> bool {
    let mut characters = segment.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    if segment.len() > 64 {
        return false;
    }
    characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
}

/// Resolve a relative path inside a run directory.
///
/// The returned path is the absolute path of the target. The path never
/// leaves the run directory and never traverses a symlink.
pub fn resolve_workspace_path(
    workspace: &Path,
    relative: &str,
) -> Result<PathBuf, ToolProtocolError> {
    if relative.trim().is_empty() {
        return Err(ToolProtocolError::workspace(
            "workspace paths must be nonempty strings",
        ));
    }
    if relative.contains('\u{0}') || relative.contains('\\') {
        return Err(ToolProtocolError::workspace(
            "workspace paths cannot contain null bytes or backslashes",
        ));
    }
    if relative.chars().count() > MAX_PATH_LENGTH {
        return Err(ToolProtocolError::workspace(
            "workspace path exceeds the bounded length",
        ));
    }
    if relative.starts_with('/') {
        return Err(ToolProtocolError::workspace(
            "workspace paths must be relative",
        ));
    }
    let segments = posix_parts(relative);
    if segments.is_empty() || segments.len() > MAX_PATH_SEGMENTS {
        return Err(ToolProtocolError::workspace(
            "workspace path has an unusable segment count",
        ));
    }
    for segment in &segments {
        if segment == ".." || !is_safe_segment(segment) {
            return Err(ToolProtocolError::workspace(
                "workspace path contains an unsafe segment",
            ));
        }
    }
    let base = resolve_path(workspace);
    if !base.is_dir() {
        return Err(ToolProtocolError::workspace(
            "workspace directory does not exist",
        ));
    }
    let mut target = base.clone();
    for segment in &segments {
        target.push(segment);
    }
    let mut current = base.clone();
    for segment in &segments {
        current.push(segment);
        if current.is_symlink() {
            return Err(ToolProtocolError::workspace(
                "workspace paths cannot traverse symlinks",
            ));
        }
    }
    let resolved = resolve_path(&target);
    if !resolved.starts_with(&base) {
        return Err(ToolProtocolError::workspace(
            "workspace path escapes its run directory",
        ));
    }
    Ok(target)
}
