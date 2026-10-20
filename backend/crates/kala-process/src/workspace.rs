//! Confined relative path and working-directory resolution.
//!
//! This is the native port of `kala_agent.tools.protocol.resolve_workspace_path`.
//! It confines a *relative path string* to an existing workspace and refuses
//! traversal segments, absolute paths, backslashes, NUL bytes, overlong paths
//! and symlink traversal. It is **not** a filesystem jail: a child that can
//! open absolute paths is only confined on Linux by the strict namespace,
//! mount and `chroot` profile, and on Windows this layer makes no filesystem
//! confinement claim at all.

use std::path::{Path, PathBuf};

use crate::error::WorkspaceError;

const MAX_PATH_SEGMENTS: usize = 32;
const MAX_PATH_LENGTH: usize = 1_024;
const MAX_SEGMENT_LENGTH: usize = 64;

/// Resolve a relative path inside an existing workspace without creating it.
///
/// The returned path is the lexical child path, as in Python; every existing
/// component is checked for symlinks and the final canonical form must remain
/// under the canonical workspace.
pub fn resolve_workspace_path(workspace: &Path, relative: &str) -> Result<PathBuf, WorkspaceError> {
    let segments = validate_relative(relative)?;
    let base = canonical_directory(workspace)?;
    let mut current = base.clone();
    for segment in &segments {
        current = current.join(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(WorkspaceError::Symlink);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(WorkspaceError::Io(error.to_string())),
        }
    }
    let resolved = resolve_lexical_or_existing(&current);
    if !resolved.starts_with(&base) {
        return Err(WorkspaceError::Escape);
    }
    Ok(current)
}

/// Resolve a relative path and require the result to be an existing directory.
pub fn resolve_workspace_directory(
    workspace: &Path,
    relative: &str,
) -> Result<PathBuf, WorkspaceError> {
    let target = resolve_workspace_path(workspace, relative)?;
    if !target.is_dir() {
        return Err(WorkspaceError::NotADirectory);
    }
    Ok(target)
}

/// Canonicalise an existing absolute directory without a path-string
/// confinement claim. This is used for the explicit working directory of a
/// process, which the caller may have resolved with
/// [`resolve_workspace_path`].
pub fn canonical_directory(path: &Path) -> Result<PathBuf, WorkspaceError> {
    if !path.is_absolute() {
        return Err(WorkspaceError::Absolute);
    }
    match std::fs::canonicalize(path) {
        Ok(canonical) if canonical.is_dir() => Ok(canonical),
        Ok(_) => Err(WorkspaceError::NotADirectory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(WorkspaceError::MissingDirectory)
        }
        Err(error) => Err(WorkspaceError::Io(error.to_string())),
    }
}

fn validate_relative(relative: &str) -> Result<Vec<String>, WorkspaceError> {
    if relative.trim().is_empty() {
        return Err(WorkspaceError::Empty);
    }
    if relative.contains('\0') || relative.contains('\\') {
        return Err(WorkspaceError::NullOrBackslash);
    }
    if relative.chars().count() > MAX_PATH_LENGTH {
        return Err(WorkspaceError::TooLong);
    }
    if relative.starts_with('/') {
        return Err(WorkspaceError::Absolute);
    }
    let mut segments = Vec::new();
    for segment in relative.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || !valid_segment(segment) {
            return Err(WorkspaceError::UnsafeSegment);
        }
        segments.push(segment.to_owned());
    }
    if segments.is_empty() || segments.len() > MAX_PATH_SEGMENTS {
        return Err(WorkspaceError::SegmentCount);
    }
    Ok(segments)
}

fn valid_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_SEGMENT_LENGTH {
        return false;
    }
    if !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn resolve_lexical_or_existing(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    match path.parent() {
        Some(parent) => match std::fs::canonicalize(parent) {
            Ok(canonical) => match path.file_name() {
                Some(name) => canonical.join(name),
                None => canonical,
            },
            Err(_) => path.to_path_buf(),
        },
        None => path.to_path_buf(),
    }
}
