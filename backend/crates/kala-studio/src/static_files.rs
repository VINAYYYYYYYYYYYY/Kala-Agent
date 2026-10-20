//! Static UI root resolution and traversal-safe file lookup.
//!
//! The root order matches the Python source where it is portable: the
//! `KALA_STUDIO_DIST` override, a frozen `studio_dist` directory beside the
//! native executable, the Electron `app/dist` resource layout, then a
//! repository-relative `studio/dist` found by walking executable ancestors.
//! Lookups canonicalize the target and refuse any path or symlink outside
//! the resolved root.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const STUDIO_DIST_ENV_VAR: &str = "KALA_STUDIO_DIST";
pub const STUDIO_DIST_INDEX: &str = "index.html";
pub const STUDIO_DIST_SUBDIR: &str = "studio_dist";
pub const STUDIO_API_PATH_PREFIX: &str = "/api";
pub const STUDIO_SPA_FALLBACK_NAME: &str = "index.html";

/// Resolve the static root for a CLI launch.
#[must_use]
pub fn resolve_static_root(
    environ: &HashMap<String, String>,
    executable_dir: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(raw) = environ.get(STUDIO_DIST_ENV_VAR) {
        if !raw.trim().is_empty() {
            if let Some(path) = expand_path(raw) {
                if path.join(STUDIO_DIST_INDEX).is_file() {
                    return Some(path);
                }
            }
        }
    }
    let executable_dir = executable_dir?;
    let nested = executable_dir.join(STUDIO_DIST_SUBDIR);
    if nested.join(STUDIO_DIST_INDEX).is_file() {
        return Some(nested);
    }
    if let Some(resources) = executable_dir.parent() {
        let electron = resources.join("app").join("dist");
        if electron.join(STUDIO_DIST_INDEX).is_file() {
            return Some(electron);
        }
    }
    for parent in executable_dir.ancestors() {
        let candidate = parent.join("studio").join("dist");
        if candidate.join(STUDIO_DIST_INDEX).is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Expand `~` and make a path absolute without requiring it to exist.
#[must_use]
pub fn expand_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = if trimmed == "~" {
        crate::boundary::home_directory()?
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        crate::boundary::home_directory()?.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    let absolute = if expanded.is_absolute() {
        normalise_lexically(&expanded)
    } else {
        normalise_lexically(&env::current_dir().ok()?.join(expanded))
    };
    Some(fs::canonicalize(&absolute).unwrap_or(absolute))
}

fn normalise_lexically(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// A configured static directory.
#[derive(Debug, Clone)]
pub struct StaticRoot {
    root: PathBuf,
    resolved: PathBuf,
}

impl StaticRoot {
    /// Configure a root only when it resolves to a directory.
    #[must_use]
    pub fn new(root: PathBuf) -> Option<Self> {
        let resolved = fs::canonicalize(&root).ok()?;
        if !resolved.is_dir() {
            return None;
        }
        Some(Self { root, resolved })
    }

    /// The configured root path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }
}

/// Lookup outcome for one GET/HEAD request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticLookup {
    File(PathBuf),
    /// The request names a path outside the root; answer 404 without an SPA
    /// fallback.
    Blocked,
    /// No static file (or fallback) exists.
    Missing,
}

/// Resolve a request path (raw, still percent-encoded) inside the root.
#[must_use]
pub fn lookup(root: &StaticRoot, raw_path: &str) -> StaticLookup {
    let cleaned = raw_path.split('?').next().unwrap_or("");
    let relative = python_normpath(cleaned);
    let relative = relative.trim_start_matches('/');
    let mut target = if relative.is_empty() {
        root.root.join(STUDIO_SPA_FALLBACK_NAME)
    } else {
        root.root.join(relative)
    };
    let resolved = match fs::canonicalize(&target) {
        Ok(resolved) => resolved,
        Err(_) => return fallback(root),
    };
    if !resolved.starts_with(&root.resolved) {
        return StaticLookup::Blocked;
    }
    if resolved.is_dir() {
        target = resolved.join(STUDIO_SPA_FALLBACK_NAME);
        let Ok(index) = fs::canonicalize(&target) else {
            return fallback(root);
        };
        if !index.starts_with(&root.resolved) {
            return StaticLookup::Blocked;
        }
        if !index.is_file() {
            return fallback(root);
        }
        return StaticLookup::File(index);
    }
    if resolved.is_file() {
        StaticLookup::File(resolved)
    } else {
        fallback(root)
    }
}

fn fallback(root: &StaticRoot) -> StaticLookup {
    let index = root.root.join(STUDIO_SPA_FALLBACK_NAME);
    let Ok(resolved) = fs::canonicalize(&index) else {
        return StaticLookup::Missing;
    };
    if !resolved.starts_with(&root.resolved) {
        return StaticLookup::Blocked;
    }
    if resolved.is_file() {
        StaticLookup::File(resolved)
    } else {
        StaticLookup::Missing
    }
}

/// A bounded approximation of `posixpath.normpath`, whose output only feeds a
/// `lstrip("/")` before the canonical containment check.
fn python_normpath(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_owned()
    } else {
        joined
    }
}

/// Protocol reservation: `/api` and everything below it never reaches the
/// static middleware.
#[must_use]
pub fn is_reserved_api_path(path: &str) -> bool {
    path == STUDIO_API_PATH_PREFIX || path.starts_with("/api/")
}

/// Guess a media type with the Python source's host-independent subset and
/// its `text/*` or `application/*` restriction.
#[must_use]
pub fn media_type_for(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "html" | "htm" => "text/html",
        "txt" | "c" | "h" => "text/plain",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "xml" => "text/xml",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "ts" => "text/vnd.trolltech.linguist",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn normpath_and_reservation_match_python_examples() {
        assert_eq!(python_normpath("//foo//bar/../baz"), "/foo/baz");
        assert_eq!(python_normpath("/a/./b"), "/a/b");
        assert!(is_reserved_api_path("/api"));
        assert!(is_reserved_api_path("/api/studio/health"));
        assert!(!is_reserved_api_path("/apix"));
        assert!(!is_reserved_api_path("/asset.txt"));
    }

    #[test]
    fn lookup_blocks_traversal_and_symlink_escape() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("dist");
        fs::create_dir(&root).unwrap();
        let mut index = fs::File::create(root.join("index.html")).unwrap();
        index.write_all(b"<html></html>").unwrap();
        let static_root = StaticRoot::new(root.clone()).unwrap();
        assert!(matches!(
            lookup(&static_root, "/missing"),
            StaticLookup::File(_)
        ));
        assert!(matches!(
            lookup(&static_root, "/../../etc/passwd"),
            StaticLookup::File(_)
        ));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", root.join("escape")).unwrap();
            assert_eq!(lookup(&static_root, "/escape"), StaticLookup::Blocked);
        }
    }
}
