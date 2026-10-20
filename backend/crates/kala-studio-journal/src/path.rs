//! Path preparation shared by the journal and the artifact store.
//!
//! The Python module resolves the caller-supplied data directory with
//! `Path.expanduser().resolve(strict=False)`, creates it with mode `0o700`
//! and then tightens the mode. This module reproduces that resolution, so the
//! `data_dir` property of the native journal names the same directory the
//! Python journal names.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The mode the data directory and the journal file are tightened to.
pub(crate) const OWNER_ONLY_DIRECTORY_MODE: u32 = 0o700;
/// The mode the journal database file is tightened to.
pub(crate) const OWNER_ONLY_FILE_MODE: u32 = 0o600;

/// The number of symbolic links `resolve_path` will follow.
const MAX_SYMLINK_DEPTH: usize = 40;

/// Expand a leading `~` the way `Path.expanduser` does.
///
/// Only the current user's home is expanded. `~user` is left alone, which is
/// what `Path.expanduser` does on Windows.
pub(crate) fn expand_user(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if text == "~" {
        return home_directory().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        if let Some(home) = home_directory() {
            return home.join(rest);
        }
        return path.to_path_buf();
    }
    path.to_path_buf()
}

/// Return the current user's home directory.
fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            if !profile.is_empty() {
                return Some(PathBuf::from(profile));
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            if !home.is_empty() {
                return Some(PathBuf::from(home));
            }
        }
    }
    None
}

/// Resolve a path the way `Path.resolve(strict=False)` does.
///
/// The result is absolute, every symbolic link that exists is followed, and a
/// path that does not exist is kept as the resolved parent plus the missing
/// name. The resolution is iterative so a long path cannot exhaust the
/// thread stack.
pub(crate) fn resolve_path(path: &Path) -> io::Result<PathBuf> {
    let mut pending: Vec<OsString> = Vec::new();
    let mut current = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    for _ in 0..MAX_SYMLINK_DEPTH {
        match fs::canonicalize(&current) {
            Ok(resolved) => {
                let mut result = resolved;
                for name in pending.iter().rev() {
                    result.push(name);
                }
                return Ok(result);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let metadata = fs::symlink_metadata(&current).ok();
                if metadata.is_some_and(|meta| meta.file_type().is_symlink()) {
                    let target = fs::read_link(&current)?;
                    current = if target.is_absolute() {
                        target
                    } else {
                        current
                            .parent()
                            .unwrap_or_else(|| Path::new(""))
                            .join(target)
                    };
                    continue;
                }
                let Some(name) = current.file_name().map(|name| name.to_os_string()) else {
                    return Err(error);
                };
                pending.push(name);
                let Some(parent) = current.parent() else {
                    return Err(error);
                };
                if parent.as_os_str().is_empty() {
                    return Err(error);
                }
                current = parent.to_path_buf();
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "too many levels of symbolic links",
    ))
}

/// Create the data directory and return its resolved path.
///
/// The directory is created with the owner-only mode and the mode is then
/// tightened, exactly like the Python module does. Parent directories are
/// created with the platform default mode, like `Path.mkdir(parents=True)`.
pub(crate) fn prepare_data_directory(path: &Path) -> io::Result<PathBuf> {
    let expanded = expand_user(path);
    if let Some(parent) = expanded.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    if fs::symlink_metadata(&expanded).is_err() {
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.mode(OWNER_ONLY_DIRECTORY_MODE);
            builder
        };
        #[cfg(not(unix))]
        let builder = fs::DirBuilder::new();
        match builder.create(&expanded) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let resolved = resolve_path(&expanded)?;
    set_owner_only_mode(&resolved, OWNER_ONLY_DIRECTORY_MODE);
    Ok(resolved)
}

/// Set an owner-only mode where the platform supports it.
///
/// The Python module skips the call on Windows and ignores an `OSError`.
pub(crate) fn set_owner_only_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}
