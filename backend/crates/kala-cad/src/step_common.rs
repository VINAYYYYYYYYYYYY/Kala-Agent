//! Kernel-independent STEP policy: limits, options and corpus normalisation.
//!
//! These items compile in both the native and the explicit-unavailable build,
//! because a limit validator and a byte normaliser are not kernel operations.
//! Geometry is never approximated here: anything that reads or writes a shape
//! lives in the native STEP module.

use crate::error::{StepImportError, StepImportErrorKind};
use crate::mesh_data::{MAX_MESH_TRIANGLES, MAX_MESH_VERTICES};

/// The ISO-10303-21 file magic.
pub const STEP_HEADER: &[u8] = b"ISO-10303-21";
/// The operator-facing STEP import ceiling variable.
pub const STEP_IMPORT_MAX_BYTES_ENV: &str = "KALA_STEP_IMPORT_MAX_BYTES";
/// The default upload ceiling: 64 MiB.
pub const DEFAULT_MAX_STEP_BYTES: u64 = 64 * 1024 * 1024;
/// The smallest usable STEP import ceiling.
pub const MIN_STEP_IMPORT_BYTES: u64 = 1;
/// The largest configurable STEP import ceiling: 512 MiB.
pub const MAX_STEP_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
/// The compatibility corpus' fixed `FILE_NAME` timestamp.
pub const FIXED_STEP_TIMESTAMP: &str = "2026-01-01T00:00:00";
/// Maximum display-warning length.
pub const MAX_WARNING_LENGTH: usize = 256;
/// The coordinate ceiling the Studio view model accepts.
pub const MAX_VIEW_COORDINATE_MM: f64 = 1_000_000.0;

/// STEP write options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepWriteOptions {
    /// Apply [`normalise_step_bytes`] to the written document.
    pub normalise: bool,
}

/// View-only STEP import settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepImportOptions {
    /// Upload ceiling in bytes, validated against
    /// [`MIN_STEP_IMPORT_BYTES`]..=[`MAX_STEP_IMPORT_BYTES`].
    pub max_step_bytes: u64,
    /// Maximum total display vertices.
    pub max_vertices: usize,
    /// Maximum total display triangles.
    pub max_triangles: usize,
}

impl Default for StepImportOptions {
    fn default() -> Self {
        Self {
            max_step_bytes: DEFAULT_MAX_STEP_BYTES,
            max_vertices: MAX_MESH_VERTICES,
            max_triangles: MAX_MESH_TRIANGLES,
        }
    }
}

/// Return a usable STEP import ceiling or the exact Python `ValueError`
/// message.
pub fn validate_step_import_max_bytes(value: u64) -> Result<u64, StepImportError> {
    if !(MIN_STEP_IMPORT_BYTES..=MAX_STEP_IMPORT_BYTES).contains(&value) {
        return Err(StepImportError::new(
            StepImportErrorKind::Config,
            format!(
                "the STEP import limit must be between {MIN_STEP_IMPORT_BYTES} and \
                 {MAX_STEP_IMPORT_BYTES} bytes"
            ),
        ));
    }
    Ok(value)
}

/// Resolve the import ceiling from an environment value.
///
/// `raw` is the value of [`STEP_IMPORT_MAX_BYTES_ENV`], when present. An absent
/// or blank value keeps `default`; a malformed or out-of-range value is refused
/// rather than silently changing a security boundary.
pub fn resolve_step_import_max_bytes(
    raw: Option<&str>,
    default: u64,
) -> Result<u64, StepImportError> {
    match raw {
        None => validate_step_import_max_bytes(default),
        Some(text) if text.trim().is_empty() => validate_step_import_max_bytes(default),
        Some(text) => {
            let value = parse_python_int(text.trim()).ok_or_else(|| {
                StepImportError::new(
                    StepImportErrorKind::Config,
                    format!("{STEP_IMPORT_MAX_BYTES_ENV} must be an integer number of bytes"),
                )
            })?;
            if value < 0 || value > i128::from(u64::MAX) {
                return Err(StepImportError::new(
                    StepImportErrorKind::Config,
                    format!(
                        "the STEP import limit must be between {MIN_STEP_IMPORT_BYTES} and \
                         {MAX_STEP_IMPORT_BYTES} bytes"
                    ),
                ));
            }
            validate_step_import_max_bytes(value as u64)
        }
    }
}

/// Resolve [`STEP_IMPORT_MAX_BYTES_ENV`] from the process environment.
///
/// This is the native equivalent of `resolve_step_import_max_bytes()` in
/// `importer.py`; a service that owns its configuration may call
/// [`resolve_step_import_max_bytes`] directly instead.
pub fn resolve_step_import_max_bytes_from_env(default: u64) -> Result<u64, StepImportError> {
    let raw = std::env::var_os(STEP_IMPORT_MAX_BYTES_ENV)
        .map(|value| value.to_string_lossy().into_owned());
    resolve_step_import_max_bytes(raw.as_deref(), default)
}

/// The compatibility corpus' STEP normalisation.
///
/// Two values change on every write and both are host facts, not product
/// facts:
///
/// * the first ISO timestamp (the `FILE_NAME` header line's write clock) is
///   replaced with [`FIXED_STEP_TIMESTAMP`];
/// * every `Open CASCADE STEP translator <version> <counter>` counter is
///   replaced with the same number of `0` digits, because OCCT numbers the
///   translated product per process.
///
/// Both replacements keep the byte length, so a recorded size still describes
/// the real response. The normalisation never touches geometry.
pub fn normalise_step_bytes(data: &[u8]) -> Vec<u8> {
    let mut output = data.to_vec();
    if let Some(offset) = find_timestamp(&output) {
        output[offset..offset + FIXED_STEP_TIMESTAMP.len()]
            .copy_from_slice(FIXED_STEP_TIMESTAMP.as_bytes());
    }
    let prefix = b"Open CASCADE STEP translator ";
    let mut cursor = 0;
    while cursor + prefix.len() < output.len() {
        let Some(relative) = find_bytes(&output[cursor..], prefix) else {
            break;
        };
        let version_start = cursor + relative + prefix.len();
        let mut scan = version_start;
        while scan < output.len() && (output[scan].is_ascii_digit() || output[scan] == b'.') {
            scan += 1;
        }
        // `[0-9.]+ ` followed by one or more digits. The version run cannot
        // contain the separating space, so the greedy match has one outcome.
        if scan < output.len() && output[scan] == b' ' {
            let digits_start = scan + 1;
            let mut digits_end = digits_start;
            while digits_end < output.len() && output[digits_end].is_ascii_digit() {
                digits_end += 1;
            }
            if digits_end > digits_start {
                output[digits_start..digits_end].fill(b'0');
            }
            cursor = digits_end.max(digits_start);
        } else {
            cursor = version_start;
        }
    }
    output
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_timestamp(data: &[u8]) -> Option<usize> {
    const LENGTH: usize = 19;
    if data.len() < LENGTH {
        return None;
    }
    (0..=data.len() - LENGTH).find(|offset| is_timestamp_at(data, *offset))
}

fn is_timestamp_at(data: &[u8], offset: usize) -> bool {
    const DIGITS: [usize; 14] = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    const LITERALS: [(usize, u8); 5] = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    DIGITS
        .iter()
        .all(|index| data[offset + index].is_ascii_digit())
        && LITERALS
            .iter()
            .all(|(index, byte)| data[offset + index] == *byte)
}

/// Parse the integer forms Python's `int()` accepts for this setting:
/// optional sign and ASCII digits, with single underscores between digits.
fn parse_python_int(text: &str) -> Option<i128> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut index = 0;
    let negative = match bytes[0] {
        b'+' => {
            index += 1;
            false
        }
        b'-' => {
            index += 1;
            true
        }
        _ => false,
    };
    if index >= bytes.len() {
        return None;
    }
    let mut value: i128 = 0;
    let mut previous_underscore = false;
    let mut saw_digit = false;
    for &byte in &bytes[index..] {
        if byte == b'_' {
            if !saw_digit || previous_underscore {
                return None;
            }
            previous_underscore = true;
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .checked_mul(10)?
            .checked_add(i128::from(byte - b'0'))?;
        previous_underscore = false;
        saw_digit = true;
    }
    if !saw_digit || previous_underscore {
        return None;
    }
    Some(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_normaliser_replaces_the_first_timestamp_and_every_counter() {
        let raw = b"ISO-10303-21;\nHEADER;\nFILE_NAME('x','2026-10-20T12:34:56',\
                    ('a'),('b'),'c','d','');\nDATA;\n\
                    #1=PRODUCT('Open CASCADE STEP translator 7.9 12','',...);\n\
                    #2=PRODUCT('Open CASCADE STEP translator 7.9 7','',...);\n\
                    #3='2026-10-20T12:34:56';";
        let normalised = normalise_step_bytes(raw);
        assert_eq!(normalised.len(), raw.len());
        let text = String::from_utf8(normalised).expect("ASCII");
        assert!(text.contains("FILE_NAME('x','2026-01-01T00:00:00'"));
        // Only the first occurrence is the FILE_NAME write clock; later ISO
        // timestamps in the body are data and are left alone.
        assert_eq!(text.matches("2026-10-20T12:34:56").count(), 1);
        assert_eq!(text.matches("2026-01-01T00:00:00").count(), 1);
        assert!(text.contains("translator 7.9 00"));
        assert!(text.contains("translator 7.9 0"));
    }

    #[test]
    fn the_normaliser_is_length_preserving_and_idempotent() {
        let raw = b"ISO-10303-21;\nFILE_NAME('x','2026-10-20T12:34:56','');\n\
                    #1=PRODUCT('Open CASCADE STEP translator 7.9 123',...);";
        let once = normalise_step_bytes(raw);
        let twice = normalise_step_bytes(&once);
        assert_eq!(once, twice);
        assert_eq!(once.len(), raw.len());
    }

    #[test]
    fn the_limit_messages_match_the_python_module() {
        assert_eq!(validate_step_import_max_bytes(1).unwrap(), 1);
        assert_eq!(
            validate_step_import_max_bytes(0).unwrap_err().message(),
            "the STEP import limit must be between 1 and 536870912 bytes"
        );
        assert_eq!(
            validate_step_import_max_bytes(MAX_STEP_IMPORT_BYTES + 1)
                .unwrap_err()
                .message(),
            "the STEP import limit must be between 1 and 536870912 bytes"
        );
        assert_eq!(
            resolve_step_import_max_bytes(Some("  "), 4096).unwrap(),
            4096
        );
        assert_eq!(
            resolve_step_import_max_bytes(Some("nope"), 4096)
                .unwrap_err()
                .message(),
            "KALA_STEP_IMPORT_MAX_BYTES must be an integer number of bytes"
        );
        assert_eq!(
            resolve_step_import_max_bytes(Some("+1_024"), 4096).unwrap(),
            1024
        );
    }
}
