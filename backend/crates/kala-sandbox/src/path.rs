//! Path helpers that reproduce CPython's `pathlib` behaviour.
//!
//! `SandboxSpec.to_payload` writes `root.as_posix()` and
//! `workspace.as_posix()`. `pathlib` normalises the text: it collapses
//! repeated separators, drops `.` components, removes a trailing separator and
//! renders the empty path as `.`. Rust's `Path` keeps every byte, so the
//! normalisation is reproduced here to keep the payload bytes identical.

use std::path::{Component, Path, PathBuf};

/// Render a path the way CPython's `PurePath.as_posix` does.
pub fn posix_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '\\' {
        normalize_native(&text.replace('\\', "/"), '/')
    } else {
        normalize_posix(&text)
    }
}

/// Render a path the way CPython's `str(Path(...))` does.
///
/// The native text keeps the platform separator; only the redundant
/// separators and `.` components are removed.
pub fn native_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '\\' {
        normalize_native(&text, '\\')
    } else {
        normalize_posix(&text)
    }
}

/// Normalise a path string with an explicit separator.
pub fn normalize_native(text: &str, separator: char) -> String {
    if text.is_empty() {
        return ".".to_owned();
    }
    let absolute = text.starts_with(separator);
    let mut parts: Vec<&str> = text
        .split(separator)
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let mut prefix = String::new();
    if absolute {
        prefix.push(separator);
    }
    if parts.is_empty() {
        return if absolute { prefix } else { ".".to_owned() };
    }
    // A Windows drive letter stays attached to the first component.
    if parts.len() > 1 && parts[0].ends_with(':') && separator == '\\' {
        prefix.push_str(parts.remove(0));
    }
    format!("{prefix}{}", parts.join(&separator.to_string()))
}

/// Normalise a POSIX path string exactly like `PurePosixPath`.
pub fn normalize_posix(text: &str) -> String {
    if text.is_empty() {
        return ".".to_owned();
    }
    // POSIX keeps exactly two leading separators as a distinct root.
    let root = if text.starts_with("//") && !text.starts_with("///") {
        "//"
    } else if text.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts: Vec<&str> = text
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.is_empty() {
        return if root.is_empty() {
            ".".to_owned()
        } else {
            root.to_owned()
        };
    }
    format!("{root}{}", parts.join("/"))
}

/// Resolve a path the way CPython's `Path.resolve(strict=False)` does.
///
/// The longest existing prefix is resolved through the filesystem, so symbolic
/// links are followed exactly like CPython does. The remaining, non-existent
/// components are appended and normalised lexically. A path that cannot be
/// resolved at all is returned in its normalised absolute form.
pub fn resolve_strict_false(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(current) => current.join(path),
            Err(_) => path.to_path_buf(),
        }
    };
    let mut prefix = absolute.clone();
    let mut trailing: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(&prefix) {
            let mut resolved = canonical;
            for component in trailing.iter().rev() {
                resolved.push(component);
            }
            return resolved;
        }
        match prefix.file_name() {
            Some(name) => {
                trailing.push(name.to_os_string());
                if !prefix.pop() {
                    break;
                }
            }
            None => break,
        }
    }
    normalize_lexically(&absolute)
}

/// Remove `.` and `..` components lexically.
pub fn normalize_lexically(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !output.pop() {
                    output.push("..");
                }
            }
            other => output.push(other.as_os_str()),
        }
    }
    if output.as_os_str().is_empty() {
        output.push(".");
    }
    output
}

/// The parent directory of a path, exactly like `os.path.dirname`.
///
/// `dirname("/usr/bin/python3")` is `"/usr/bin"` and `dirname("python3")` is
/// the empty string.
pub fn dirname(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) => parent.to_path_buf(),
        None => PathBuf::new(),
    }
}

/// Whether a path string starts with `/`, the literal Python check.
///
/// The Python module tests `str(root).startswith("/")` rather than
/// `Path(root).is_absolute()`. The difference is observable on Windows, where
/// `C:\\work` is absolute but does not start with a separator, and where
/// `/work` is not absolute but does start with one. The literal check is
/// preserved so the refusal boundary is identical.
pub fn has_absolute_text(path: &Path) -> bool {
    path.to_string_lossy().starts_with('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_like_pure_posix_path() {
        assert_eq!(normalize_posix(""), ".");
        assert_eq!(normalize_posix("."), ".");
        assert_eq!(normalize_posix("/"), "/");
        assert_eq!(normalize_posix("//"), "//");
        assert_eq!(normalize_posix("//a/b"), "//a/b");
        assert_eq!(normalize_posix("a//b/./c"), "a/b/c");
        assert_eq!(normalize_posix("/a/../b"), "/a/../b");
        assert_eq!(normalize_posix("/usr/lib/"), "/usr/lib");
    }

    #[test]
    fn dirname_matches_python() {
        assert_eq!(
            dirname(Path::new("/usr/bin/python3")),
            PathBuf::from("/usr/bin")
        );
        assert_eq!(dirname(Path::new("python3")), PathBuf::new());
    }
}
