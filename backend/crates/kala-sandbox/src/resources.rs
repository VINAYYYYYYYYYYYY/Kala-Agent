//! The host paths a jailed worker needs in order to start at all.
//!
//! The Python module derives those paths from the running interpreter
//! (`sys.executable`, `sys.prefix`, `sysconfig` search paths). A native
//! worker has no interpreter, so the facts are supplied explicitly through
//! [`InterpreterLayout`]: the algorithm is unchanged and the differential test
//! feeds the live Python interpreter's own facts through the same code.

use std::path::{Path, PathBuf};

use crate::constants::{OPTIONAL_SYSTEM_DIRECTORIES, OPTIONAL_SYSTEM_FILES};
use crate::path::{dirname, native_text, posix_text, resolve_strict_false};

/// The interpreter facts the Python module reads from `sys` and `sysconfig`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InterpreterLayout {
    /// `sys.executable`.
    pub executable: PathBuf,
    /// `sys.prefix`.
    pub prefix: PathBuf,
    /// `sys.base_prefix`.
    pub base_prefix: PathBuf,
    /// `sys.exec_prefix`.
    pub exec_prefix: PathBuf,
    /// `sys.base_exec_prefix`.
    pub base_exec_prefix: PathBuf,
    /// `os.path.dirname(os.__file__)`: the directory of the package root.
    pub module_directory: PathBuf,
    /// `sysconfig.get_paths()["stdlib"]`.
    pub stdlib: Option<PathBuf>,
    /// `sysconfig.get_paths()["platstdlib"]`.
    pub platstdlib: Option<PathBuf>,
    /// `sysconfig.get_paths()["purelib"]`.
    pub purelib: Option<PathBuf>,
    /// `sysconfig.get_paths()["platlib"]`.
    pub platlib: Option<PathBuf>,
}

impl InterpreterLayout {
    /// The native analogue of the interpreter layout.
    ///
    /// A native worker is a single executable, so the "prefix" is the
    /// directory that holds it and there is no standard library or site
    /// directory. The layout is used only to decide which host paths a jail
    /// has to bind read-only.
    pub fn current() -> Self {
        let executable = std::env::current_exe().unwrap_or_default();
        let directory = dirname(&executable);
        Self {
            prefix: directory.clone(),
            base_prefix: directory.clone(),
            exec_prefix: directory.clone(),
            base_exec_prefix: directory,
            module_directory: dirname(&executable),
            executable,
            stdlib: None,
            platstdlib: None,
            purelib: None,
            platlib: None,
        }
    }
}

/// The host paths a jailed worker needs in order to start at all.
///
/// The order, the de-duplication and the "first non-empty wins" rule are the
/// Python ones.
pub fn child_resource_paths(layout: &InterpreterLayout) -> Vec<String> {
    let executable = realpath(&layout.executable);
    let mut candidates: Vec<String> = Vec::new();
    let push = |value: &Path, candidates: &mut Vec<String>| {
        let text = native_text(value);
        if !text.is_empty() && !candidates.contains(&text) {
            candidates.push(text);
        }
    };
    push(&layout.executable, &mut candidates);
    push(&executable, &mut candidates);
    push(&dirname(&executable), &mut candidates);
    push(&dirname(&layout.executable), &mut candidates);
    push(&layout.prefix, &mut candidates);
    push(&layout.base_prefix, &mut candidates);
    push(&layout.exec_prefix, &mut candidates);
    push(&layout.base_exec_prefix, &mut candidates);
    push(&layout.module_directory, &mut candidates);
    for candidate in [
        layout.stdlib.as_ref(),
        layout.platstdlib.as_ref(),
        layout.purelib.as_ref(),
        layout.platlib.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        push(candidate, &mut candidates);
    }
    candidates
}

/// The read-only host paths a jailed worker must be able to reach.
///
/// The child resource paths of the running process are used, followed by the
/// explicit import roots.
pub fn resolve_system_bindings(import_roots: &[PathBuf]) -> Vec<String> {
    let child_paths = child_resource_paths(&InterpreterLayout::current());
    resolve_system_bindings_with(import_roots, &child_paths)
}

/// The read-only host paths a jailed worker must be able to reach.
///
/// This is the exact Python algorithm with the child resource paths supplied
/// explicitly, so the differential test can inject the live interpreter's own
/// paths.
pub fn resolve_system_bindings_with(
    import_roots: &[PathBuf],
    child_paths: &[String],
) -> Vec<String> {
    let mut resolved: Vec<String> = Vec::new();
    let candidates: Vec<String> = child_paths
        .iter()
        .cloned()
        .chain(import_roots.iter().map(|root| native_text(root)))
        .collect();
    for candidate in candidates {
        let target = Path::new(&candidate);
        if !target.exists() {
            continue;
        }
        for path in [target.to_path_buf(), resolve_strict_false(target)] {
            let text = posix_text(&path);
            if !resolved.contains(&text) {
                resolved.push(text);
            }
        }
    }
    for candidate in OPTIONAL_SYSTEM_DIRECTORIES {
        if Path::new(candidate).is_dir() && !resolved.iter().any(|value| value == candidate) {
            resolved.push((*candidate).to_owned());
        }
    }
    for candidate in OPTIONAL_SYSTEM_FILES {
        if Path::new(candidate).is_file() && !resolved.iter().any(|value| value == candidate) {
            resolved.push((*candidate).to_owned());
        }
    }
    resolved
}

/// Resolve a path the way `os.path.realpath` does, without requiring it to
/// exist.
pub fn realpath(path: &Path) -> PathBuf {
    resolve_strict_false(path)
}
