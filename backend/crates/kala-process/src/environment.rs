//! Minimal, protected child environments.
//!
//! The Python executor owns every key that bounds a child. This module ports
//! that contract exactly for the transitional Python worker
//! ([`build_child_environment`]) and provides a native-worker constructor that
//! does not carry interpreter variables ([`ChildEnvironment::minimal`]).
//!
//! The child never inherits the parent environment implicitly. The caller
//! supplies a [`ChildEnvironment`], and every platform spawn path passes that
//! environment explicitly.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::error::EnvironmentError;

/// Python tool protocol version, retained for compatibility payloads.
pub const TOOL_PROTOCOL_VERSION: &str = "1.0";

/// Native process protocol version.
pub const PROCESS_PROTOCOL_VERSION: &str = "1.0";

/// Keys the Python executor owns and no profile may replace.
pub const PROTECTED_CHILD_ENVIRONMENT_KEYS: &[&str] = &[
    "HOME",
    "KALA_TOOL_PROTOCOL",
    "KALA_PROCESS_PROTOCOL",
    "LANG",
    "PATH",
    "PYTHONDONTWRITEBYTECODE",
    "PYTHONHASHSEED",
    "PYTHONIOENCODING",
    "PYTHONPATH",
    "PYTHONNOUSERSITE",
    "PYTHONSAFEPATH",
    "TMP",
    "TMPDIR",
    "TEMP",
    "USERPROFILE",
];

/// The exact key set produced by the Python `build_child_environment` helper.
pub const CHILD_ENVIRONMENT_KEYS: &[&str] = &[
    "HOME",
    "KALA_TOOL_PROTOCOL",
    "LANG",
    "PATH",
    "PYTHONDONTWRITEBYTECODE",
    "PYTHONHASHSEED",
    "PYTHONIOENCODING",
    "PYTHONNOUSERSITE",
    "PYTHONPATH",
    "PYTHONSAFEPATH",
    "TMPDIR",
];

/// One environment variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentEntry {
    /// The variable name.
    pub key: String,
    /// The variable value.
    pub value: String,
}

impl EnvironmentEntry {
    /// Create a validated entry.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Result<Self, EnvironmentError> {
        let key = key.into();
        let value = value.into();
        validate_key(&key)?;
        validate_value(&value)?;
        Ok(Self { key, value })
    }
}

/// A complete, explicit child environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildEnvironment {
    entries: Vec<EnvironmentEntry>,
}

impl ChildEnvironment {
    /// Build from entries, rejecting duplicates (case-insensitively on
    /// Windows) and malformed names/values.
    pub fn from_entries(
        entries: impl IntoIterator<Item = EnvironmentEntry>,
    ) -> Result<Self, EnvironmentError> {
        let mut sorted: Vec<EnvironmentEntry> = entries.into_iter().collect();
        // `EnvironmentEntry` has public fields, so every aggregate constructor
        // must re-validate each entry. This is the boundary that prevents a
        // caller-built `A=B` key or an embedded NUL from reaching a platform
        // environment block.
        for entry in &sorted {
            validate_key(&entry.key)?;
            validate_value(&entry.value)?;
        }
        sorted.sort_by(|left, right| compare_keys(&left.key, &right.key));
        for pair in sorted.windows(2) {
            if keys_equal(&pair[0].key, &pair[1].key) {
                return Err(EnvironmentError::DuplicateKey);
            }
        }
        Ok(Self { entries: sorted })
    }

    /// Re-check the aggregate invariant before a platform environment block is
    /// built. Public fields on [`EnvironmentEntry`] make validation at the
    /// aggregate boundary mandatory; this is a defensive second check.
    pub(crate) fn validate(&self) -> Result<(), EnvironmentError> {
        for entry in &self.entries {
            validate_key(&entry.key)?;
            validate_value(&entry.value)?;
        }
        Ok(())
    }

    /// The production native-worker environment.
    ///
    /// It binds the home and temporary directory to the workspace, uses a
    /// fixed UTF-8 locale and a fixed minimal PATH, and adds no interpreter or
    /// secret variables.
    pub fn minimal(workspace: &Path) -> Result<Self, EnvironmentError> {
        let workspace = resolved_workspace(workspace)?;
        let mut entries = vec![
            entry("HOME", workspace.as_os_str())?,
            entry("KALA_PROCESS_PROTOCOL", PROCESS_PROTOCOL_VERSION)?,
            entry("LANG", "C.UTF-8")?,
            entry("PATH", default_path())?,
            entry("TMPDIR", workspace.as_os_str())?,
        ];
        if cfg!(windows) {
            entries.push(entry("USERPROFILE", workspace.as_os_str())?);
            entries.push(entry("TMP", workspace.as_os_str())?);
            entries.push(entry("TEMP", workspace.as_os_str())?);
        }
        Self::from_entries(entries)
    }

    /// The exact Python `build_child_environment` port, used only for
    /// transitional Python tool workers.
    ///
    /// The native worker path does not call this function. It exists so the
    /// compatibility boundary is explicit and testable, not hidden in the
    /// product process launcher.
    pub fn legacy_python_worker(
        workspace: &Path,
        import_roots: &[PathBuf],
    ) -> Result<Self, EnvironmentError> {
        build_child_environment(workspace, import_roots, &[])
    }

    /// Add or replace variables that are not owned by the supervisor.
    pub fn with_extra(mut self, extra: &[(String, String)]) -> Result<Self, EnvironmentError> {
        for (key, value) in extra {
            validate_key(key)?;
            validate_value(value)?;
            if is_protected(key) {
                return Err(EnvironmentError::ProtectedKey(key.clone()));
            }
            self.entries.retain(|entry| !keys_equal(&entry.key, key));
            self.entries.push(EnvironmentEntry {
                key: key.clone(),
                value: value.clone(),
            });
        }
        self.entries
            .sort_by(|left, right| compare_keys(&left.key, &right.key));
        Ok(self)
    }

    /// Add variables only from an explicit allow-list. This is the equivalent
    /// of the Python `SandboxProfile` rule that accepts only non-authority
    /// keys such as `TZ`.
    pub fn with_allowlisted_extra(
        self,
        extra: &BTreeMap<String, String>,
        allowed: &[&str],
    ) -> Result<Self, EnvironmentError> {
        for key in extra.keys() {
            validate_key(key)?;
            if is_protected(key) {
                return Err(EnvironmentError::ProtectedKey(key.clone()));
            }
            if !allowed.iter().any(|candidate| candidate == key) {
                return Err(EnvironmentError::DisallowedKey(key.clone()));
            }
        }
        let pairs: Vec<(String, String)> = extra
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        self.with_extra(&pairs)
    }

    /// The environment entries in platform sort order.
    pub fn entries(&self) -> &[EnvironmentEntry] {
        &self.entries
    }

    /// Look up a key exactly.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.value.as_str())
    }

    /// Number of variables.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the environment is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The exact Python `build_child_environment` contract.
///
/// Keys are fixed by the supervisor; `extra` may not replace any protected
/// key. Unknown non-protected keys are accepted here because the Python
/// profile-level allow-list is enforced by `SandboxProfile`, not by this
/// helper. See [`ChildEnvironment::with_allowlisted_extra`] for the profile
/// boundary.
pub fn build_child_environment(
    workspace: &Path,
    import_roots: &[PathBuf],
    extra: &[(String, String)],
) -> Result<ChildEnvironment, EnvironmentError> {
    let workspace = resolved_workspace(workspace)?;
    let resolved_roots: Result<Vec<PathBuf>, EnvironmentError> = import_roots
        .iter()
        .map(|root| resolved_workspace(root))
        .collect();
    let roots: Vec<String> = resolved_roots?
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let separator = if cfg!(windows) { ";" } else { ":" };
    let entries = vec![
        entry("HOME", workspace.as_os_str())?,
        entry("KALA_TOOL_PROTOCOL", TOOL_PROTOCOL_VERSION)?,
        entry("LANG", "C.UTF-8")?,
        entry("PATH", default_path())?,
        entry("PYTHONDONTWRITEBYTECODE", "1")?,
        entry("PYTHONHASHSEED", "0")?,
        entry("PYTHONIOENCODING", "utf-8")?,
        entry("PYTHONNOUSERSITE", "1")?,
        entry("PYTHONPATH", roots.join(separator))?,
        entry("PYTHONSAFEPATH", "1")?,
        entry("TMPDIR", workspace.as_os_str())?,
    ];
    let mut environment = ChildEnvironment::from_entries(entries)?;
    for (key, value) in extra {
        validate_key(key)?;
        validate_value(value)?;
        if is_protected(key) {
            return Err(EnvironmentError::ProtectedKey(key.clone()));
        }
        environment
            .entries
            .retain(|entry| !keys_equal(&entry.key, key));
        environment.entries.push(EnvironmentEntry {
            key: key.clone(),
            value: value.clone(),
        });
    }
    environment
        .entries
        .sort_by(|left, right| compare_keys(&left.key, &right.key));
    Ok(environment)
}

/// Whether a key is owned by the supervisor.
pub fn is_protected(key: &str) -> bool {
    PROTECTED_CHILD_ENVIRONMENT_KEYS.iter().any(|candidate| {
        if cfg!(windows) {
            candidate.eq_ignore_ascii_case(key)
        } else {
            *candidate == key
        }
    })
}

/// Validate an environment key using the Python regex
/// `^[A-Za-z_][A-Za-z0-9_]{0,63}$`.
pub fn validate_key(key: &str) -> Result<(), EnvironmentError> {
    let bytes = key.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(EnvironmentError::InvalidKey);
    }
    if !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return Err(EnvironmentError::InvalidKey);
    }
    if !bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        return Err(EnvironmentError::InvalidKey);
    }
    Ok(())
}

fn validate_value(value: &str) -> Result<(), EnvironmentError> {
    if value.contains('\0') {
        return Err(EnvironmentError::NullValue);
    }
    Ok(())
}

fn entry(
    key: &str,
    value: impl AsRef<std::ffi::OsStr>,
) -> Result<EnvironmentEntry, EnvironmentError> {
    let value = value.as_ref().to_string_lossy().into_owned();
    EnvironmentEntry::new(key, value)
}

fn default_path() -> &'static str {
    if cfg!(windows) {
        "C:\\Windows\\System32;C:\\Windows"
    } else {
        "/usr/local/bin:/usr/bin:/bin"
    }
}

fn keys_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn compare_keys(left: &str, right: &str) -> std::cmp::Ordering {
    if cfg!(windows) {
        left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
    } else {
        left.cmp(right)
    }
}

fn resolved_workspace(path: &Path) -> Result<PathBuf, EnvironmentError> {
    let absolute = if path.is_absolute() {
        lexical_normalize(path)
    } else {
        let current = std::env::current_dir()
            .map_err(|error| EnvironmentError::Workspace(error.to_string()))?;
        lexical_normalize(&current.join(path))
    };
    std::fs::canonicalize(&absolute).or(Ok(absolute))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}
