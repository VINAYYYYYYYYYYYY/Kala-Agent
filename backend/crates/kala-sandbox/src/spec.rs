//! The resolved description of the jail a child must build.
//!
//! The port is exact: the payload keys, their order, the malformed-payload
//! refusal and the error precedence of [`build_sandbox_spec`] match
//! `kala_agent.tools.sandbox`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::constants::{DEVICE_NODES, SCRATCH_PATHS, STRICT_REQUIRED_CONTROLS};
use crate::error::SandboxSetupError;
use crate::json::JsonValue;
use crate::path::{has_absolute_text, posix_text, resolve_strict_false};

/// A resolved, minimal description of the jail a child must build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxSpec {
    /// The controls this specification requests.
    pub controls: BTreeSet<String>,
    /// The host-side jail root.
    pub root: PathBuf,
    /// The host-side workspace that becomes writable inside the jail.
    pub workspace: PathBuf,
    /// The host paths that are bind-mounted read-only.
    pub readonly_bindings: Vec<String>,
    /// The host paths that are bind-mounted read-write.
    pub writable_bindings: Vec<String>,
    /// The device nodes that are bind-mounted into the jail.
    pub device_bindings: Vec<String>,
    /// The paths that receive a private tmpfs.
    pub scratch_paths: Vec<String>,
    /// The size of each scratch tmpfs, in bytes.
    pub scratch_bytes: i64,
}

impl SandboxSpec {
    /// Build a specification without the validation of
    /// [`build_sandbox_spec`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        controls: BTreeSet<String>,
        root: PathBuf,
        workspace: PathBuf,
        readonly_bindings: Vec<String>,
        writable_bindings: Vec<String>,
        device_bindings: Vec<String>,
        scratch_paths: Vec<String>,
        scratch_bytes: i64,
    ) -> Self {
        Self {
            controls,
            root,
            workspace,
            readonly_bindings,
            writable_bindings,
            device_bindings,
            scratch_paths,
            scratch_bytes,
        }
    }

    /// The exact Python payload shape, with the Python key order.
    pub fn to_payload(&self) -> JsonValue {
        JsonValue::object(vec![
            (
                "controls".to_owned(),
                JsonValue::Array(
                    self.controls
                        .iter()
                        .map(|control| JsonValue::Text(control.clone()))
                        .collect(),
                ),
            ),
            (
                "device_bindings".to_owned(),
                JsonValue::Array(
                    self.device_bindings
                        .iter()
                        .map(|binding| JsonValue::Text(binding.clone()))
                        .collect(),
                ),
            ),
            (
                "readonly_bindings".to_owned(),
                JsonValue::Array(
                    self.readonly_bindings
                        .iter()
                        .map(|binding| JsonValue::Text(binding.clone()))
                        .collect(),
                ),
            ),
            ("root".to_owned(), JsonValue::Text(posix_text(&self.root))),
            (
                "scratch_bytes".to_owned(),
                JsonValue::Integer(self.scratch_bytes),
            ),
            (
                "scratch_paths".to_owned(),
                JsonValue::Array(
                    self.scratch_paths
                        .iter()
                        .map(|path| JsonValue::Text(path.clone()))
                        .collect(),
                ),
            ),
            (
                "writable_bindings".to_owned(),
                JsonValue::Array(
                    self.writable_bindings
                        .iter()
                        .map(|binding| JsonValue::Text(binding.clone()))
                        .collect(),
                ),
            ),
            (
                "workspace".to_owned(),
                JsonValue::Text(posix_text(&self.workspace)),
            ),
        ])
    }

    /// The payload rendered exactly like `json.dumps(payload, ensure_ascii=True)`.
    pub fn to_json(&self) -> String {
        self.to_payload().dump()
    }

    /// Decode a specification, refusing every malformed payload.
    ///
    /// The refusal message is the Python one: `the sandbox specification is
    /// malformed`.
    pub fn from_payload(payload: &Value) -> Result<Self, SandboxSetupError> {
        let object = payload
            .as_object()
            .ok_or_else(SandboxSetupError::malformed)?;
        let controls =
            string_array(object.get("controls")).ok_or_else(SandboxSetupError::malformed)?;
        let root = path_value(object.get("root")).ok_or_else(SandboxSetupError::malformed)?;
        let workspace =
            path_value(object.get("workspace")).ok_or_else(SandboxSetupError::malformed)?;
        let readonly_bindings = string_array(object.get("readonly_bindings"))
            .ok_or_else(SandboxSetupError::malformed)?;
        let writable_bindings = string_array(object.get("writable_bindings"))
            .ok_or_else(SandboxSetupError::malformed)?;
        let device_bindings =
            string_array(object.get("device_bindings")).ok_or_else(SandboxSetupError::malformed)?;
        let scratch_paths =
            string_array(object.get("scratch_paths")).ok_or_else(SandboxSetupError::malformed)?;
        let scratch_bytes =
            integer_value(object.get("scratch_bytes")).ok_or_else(SandboxSetupError::malformed)?;
        Ok(Self {
            controls: controls.into_iter().collect(),
            root,
            workspace,
            readonly_bindings,
            writable_bindings,
            device_bindings,
            scratch_paths,
            scratch_bytes,
        })
    }

    /// Whether every required strict control is present.
    pub fn has_required_controls(&self) -> bool {
        STRICT_REQUIRED_CONTROLS
            .iter()
            .all(|control| self.controls.contains(*control))
    }
}

/// Build a strict specification after gating on every required control.
///
/// The error precedence is the Python one: a missing control is reported
/// before a relative path, and a relative path before anything else.
pub fn build_sandbox_spec(
    controls: &BTreeSet<String>,
    root: &Path,
    workspace: &Path,
    import_roots: &[PathBuf],
    scratch_bytes: i64,
) -> Result<SandboxSpec, SandboxSetupError> {
    let child_paths =
        crate::resources::child_resource_paths(&crate::resources::InterpreterLayout::current());
    build_sandbox_spec_with(
        controls,
        root,
        workspace,
        import_roots,
        scratch_bytes,
        &child_paths,
    )
}

/// Build a strict specification with the child resource paths supplied.
///
/// This is the exact Python algorithm; the child resource paths are the host
/// paths a jailed worker needs, and a native worker derives them from its own
/// layout while a Python worker derives them from its interpreter.
pub fn build_sandbox_spec_with(
    controls: &BTreeSet<String>,
    root: &Path,
    workspace: &Path,
    import_roots: &[PathBuf],
    scratch_bytes: i64,
    child_paths: &[String],
) -> Result<SandboxSpec, SandboxSetupError> {
    let missing: Vec<&str> = STRICT_REQUIRED_CONTROLS
        .iter()
        .copied()
        .filter(|control| !controls.contains(*control))
        .collect();
    if !missing.is_empty() {
        return Err(SandboxSetupError::new(format!(
            "the strict profile requires controls that were not probed: {}",
            missing.join(", ")
        )));
    }
    if !has_absolute_text(root) || !has_absolute_text(workspace) {
        return Err(SandboxSetupError::new("sandbox paths must be absolute"));
    }
    let workspace_resolved = resolve_strict_false(workspace);
    Ok(SandboxSpec {
        controls: controls.clone(),
        root: root.to_path_buf(),
        workspace: workspace.to_path_buf(),
        readonly_bindings: crate::resources::resolve_system_bindings_with(
            import_roots,
            child_paths,
        ),
        writable_bindings: vec![posix_text(&workspace_resolved)],
        device_bindings: DEVICE_NODES
            .iter()
            .filter(|candidate| Path::new(candidate).exists())
            .map(|candidate| (*candidate).to_owned())
            .collect(),
        scratch_paths: SCRATCH_PATHS
            .iter()
            .map(|path| (*path).to_owned())
            .collect(),
        scratch_bytes,
    })
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    let items = value?.as_array()?;
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        values.push(item.as_str()?.to_owned());
    }
    Some(values)
}

fn path_value(value: Option<&Value>) -> Option<PathBuf> {
    let text = value?.as_str()?;
    Some(python_path(text))
}

/// Build a path the way CPython's `Path(text)` does.
///
/// `Path("")` is `PosixPath('.')` in CPython; Rust keeps the empty string.
pub fn python_path(text: &str) -> PathBuf {
    if text.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(text)
    }
}

/// Read an integer with CPython's `int()` semantics.
///
/// `int()` accepts an integer, a float (truncated towards zero), a boolean and
/// a decimal string with optional surrounding whitespace and single
/// underscores between digits. A non-finite float and an unparsable string are
/// refused, exactly like CPython's `ValueError`.
pub fn integer_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Some(integer)
            } else if let Some(float) = number.as_f64() {
                if !float.is_finite() {
                    return None;
                }
                let truncated = float.trunc();
                if truncated < i64::MIN as f64 || truncated > i64::MAX as f64 {
                    return None;
                }
                Some(truncated as i64)
            } else {
                None
            }
        }
        Value::Bool(value) => Some(i64::from(*value)),
        Value::String(text) => python_int(text),
        _ => None,
    }
}

/// Parse a decimal integer string with CPython's `int()` rules.
pub fn python_int(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    if digits.is_empty() {
        return None;
    }
    let mut cleaned = String::with_capacity(digits.len());
    let mut previous_underscore = true;
    for character in digits.chars() {
        if character == '_' {
            if previous_underscore {
                return None;
            }
            previous_underscore = true;
            continue;
        }
        if !character.is_ascii_digit() {
            return None;
        }
        previous_underscore = false;
        cleaned.push(character);
    }
    if previous_underscore {
        return None;
    }
    let magnitude: i64 = cleaned.parse().ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn payload_keeps_python_key_order() {
        let spec = SandboxSpec::new(
            ["user_namespace".to_owned()].into_iter().collect(),
            PathBuf::from("/root"),
            PathBuf::from("/ws"),
            vec!["/usr".to_owned()],
            vec!["/ws".to_owned()],
            vec!["/dev/null".to_owned()],
            vec!["/tmp".to_owned()],
            1 << 20,
        );
        assert_eq!(
            spec.to_json(),
            "{\"controls\": [\"user_namespace\"], \"device_bindings\": [\"/dev/null\"], \
             \"readonly_bindings\": [\"/usr\"], \"root\": \"/root\", \"scratch_bytes\": 1048576, \
             \"scratch_paths\": [\"/tmp\"], \"writable_bindings\": [\"/ws\"], \"workspace\": \"/ws\"}"
        );
    }

    #[test]
    fn rejects_a_malformed_payload() {
        let error = SandboxSpec::from_payload(&json!({"controls": ["a"]})).unwrap_err();
        assert_eq!(error.detail(), "the sandbox specification is malformed");
    }

    #[test]
    fn reads_python_integers() {
        assert_eq!(python_int("  1_0 "), Some(10));
        assert_eq!(python_int("1__0"), None);
        assert_eq!(python_int(""), None);
        assert_eq!(python_int("-5"), Some(-5));
        assert_eq!(integer_value(Some(&json!(1.9))), Some(1));
        assert_eq!(integer_value(Some(&json!("4096"))), Some(4096));
        assert_eq!(integer_value(Some(&json!("not-a-number"))), None);
    }
}
