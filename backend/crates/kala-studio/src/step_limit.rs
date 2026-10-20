//! The STEP import ceiling shared by the front door and the Python worker
//! (ADR-0011).
//!
//! A provider-built STEP assembly is much larger than an ordinary Studio
//! request body, so the view-only import has its own ceiling instead of the
//! shared 256 KiB Studio limit. The ceiling is one setting:
//! `KALA_STEP_IMPORT_MAX_BYTES`.
//!
//! The Python worker owns the setting: it reads the variable, validates it
//! and refuses to start when the value is unusable. The front door must
//! never disagree with the worker, so this module is the exact port of the
//! worker's resolution:
//!
//! | Input | Result |
//! |---|---|
//! | absent or blank | the 64 MiB default |
//! | an integer in `1 ..= 512 MiB` | that value |
//! | anything else | a startup refusal, with the worker's own message |
//!
//! The front door resolves the value once, enforces it on the public import
//! route and hands the **resolved** value to the worker process, so the two
//! sides always accept exactly the same body.
//!
//! # The integer grammar
//!
//! The worker parses the value with CPython's `int()`, which accepts an
//! optional sign, surrounding whitespace and single underscores between
//! digits (`1_048_576` is 1 MiB). [`parse_python_integer`] implements that
//! grammar for ASCII digits. CPython also accepts non-ASCII decimal
//! digits; this port refuses them with the same "must be an integer number
//! of bytes" message, which is a documented difference.

use std::collections::BTreeMap;

/// The environment variable that carries the STEP import ceiling.
pub const STEP_IMPORT_MAX_BYTES_ENV: &str = "KALA_STEP_IMPORT_MAX_BYTES";

/// The default STEP import ceiling: 64 MiB.
pub const DEFAULT_MAX_STEP_BYTES: usize = 64 * 1024 * 1024;

/// The smallest accepted STEP import ceiling: one byte.
pub const MIN_STEP_IMPORT_BYTES: usize = 1;

/// The largest accepted STEP import ceiling: 512 MiB.
pub const MAX_STEP_IMPORT_BYTES: usize = 512 * 1024 * 1024;

/// The worker's message for a value that is not an integer.
pub const STEP_IMPORT_LIMIT_TYPE_MESSAGE: &str =
    "the STEP import limit must be an integer number of bytes";

/// The worker's message for a value outside the accepted range.
pub const STEP_IMPORT_LIMIT_RANGE_MESSAGE: &str =
    "the STEP import limit must be between 1 and 536870912 bytes";

/// The worker's message for an unusable environment value.
pub const STEP_IMPORT_ENV_TYPE_MESSAGE: &str =
    "KALA_STEP_IMPORT_MAX_BYTES must be an integer number of bytes";

/// Validate one STEP import ceiling.
///
/// The value must be an integer between [`MIN_STEP_IMPORT_BYTES`] and
/// [`MAX_STEP_IMPORT_BYTES`], exactly like the worker's
/// `validate_step_import_max_bytes`.
///
/// # Errors
///
/// Returns the worker's own message: the range message for a value outside
/// the bounds, the type message for a value that is not an integer.
pub fn validate_step_import_max_bytes(value: i64) -> Result<usize, String> {
    let Ok(limit) = usize::try_from(value) else {
        return Err(STEP_IMPORT_LIMIT_RANGE_MESSAGE.to_owned());
    };
    if !(MIN_STEP_IMPORT_BYTES..=MAX_STEP_IMPORT_BYTES).contains(&limit) {
        return Err(STEP_IMPORT_LIMIT_RANGE_MESSAGE.to_owned());
    }
    Ok(limit)
}

/// Resolve the STEP import ceiling from an environment view.
///
/// An absent or blank value keeps the 64 MiB default. A malformed value
/// refuses startup with the worker's own message instead of silently
/// changing a security boundary.
///
/// # Errors
///
/// Returns the worker's own message: the environment message for a value
/// that is not an integer, the range message for a value outside the bounds.
pub fn resolve_step_import_max_bytes(environ: &BTreeMap<String, String>) -> Result<usize, String> {
    let Some(raw) = environ.get(STEP_IMPORT_MAX_BYTES_ENV) else {
        return Ok(DEFAULT_MAX_STEP_BYTES);
    };
    if raw.trim().is_empty() {
        return Ok(DEFAULT_MAX_STEP_BYTES);
    }
    let Some((negative, digits)) = python_integer_parts(raw.trim()) else {
        return Err(STEP_IMPORT_ENV_TYPE_MESSAGE.to_owned());
    };
    // CPython's integers are unbounded: a value that does not fit in 64 bits
    // is certainly above the accepted maximum, so it is a range refusal and
    // not a malformed value.
    let Some(magnitude) = digits.parse::<i64>().ok() else {
        return Err(STEP_IMPORT_LIMIT_RANGE_MESSAGE.to_owned());
    };
    let value = if negative {
        magnitude.checked_neg().unwrap_or(i64::MAX)
    } else {
        magnitude
    };
    validate_step_import_max_bytes(value)
}

/// Parse an integer with CPython's `int()` grammar.
///
/// The grammar accepts an optional sign, ASCII decimal digits and single
/// underscores between digits. Surrounding whitespace is the caller's
/// business: the worker strips the value before parsing it.
///
/// # Errors
///
/// Returns `None` when the text is not an integer in that grammar or when
/// the integer does not fit in an `i64`.
pub fn parse_python_integer(text: &str) -> Option<i64> {
    let (negative, digits) = python_integer_parts(text)?;
    let magnitude = digits.parse::<i64>().ok()?;
    Some(if negative {
        magnitude.checked_neg()?
    } else {
        magnitude
    })
}

/// Split an integer into its sign and its digits, with CPython's grammar.
///
/// The digits are returned verbatim, so an integer that is larger than an
/// `i64` is still recognised as an integer.
fn python_integer_parts(text: &str) -> Option<(bool, String)> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut negative = false;
    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        negative = bytes[index] == b'-';
        index += 1;
    }
    let mut digits = String::new();
    let mut previous_was_digit = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_digit() {
            digits.push(byte as char);
            previous_was_digit = true;
            index += 1;
            continue;
        }
        if byte == b'_' {
            // CPython allows a single underscore between two digits only.
            if !previous_was_digit {
                return None;
            }
            previous_was_digit = false;
            index += 1;
            continue;
        }
        return None;
    }
    if !previous_was_digit {
        // No digits at all, or a trailing underscore.
        return None;
    }
    Some((negative, digits))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environ(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn an_absent_value_keeps_the_64_mib_default() {
        assert_eq!(
            resolve_step_import_max_bytes(&BTreeMap::new()).unwrap(),
            DEFAULT_MAX_STEP_BYTES
        );
        assert_eq!(DEFAULT_MAX_STEP_BYTES, 67_108_864);
    }

    #[test]
    fn a_blank_value_keeps_the_default() {
        for raw in ["", "   ", "\t"] {
            let view = environ(&[(STEP_IMPORT_MAX_BYTES_ENV, raw)]);
            assert_eq!(
                resolve_step_import_max_bytes(&view).unwrap(),
                DEFAULT_MAX_STEP_BYTES,
                "{raw:?}"
            );
        }
    }

    #[test]
    fn an_override_is_used_verbatim() {
        let view = environ(&[(STEP_IMPORT_MAX_BYTES_ENV, "1048576")]);
        assert_eq!(resolve_step_import_max_bytes(&view).unwrap(), 1_048_576);
        let view = environ(&[(STEP_IMPORT_MAX_BYTES_ENV, "1")]);
        assert_eq!(resolve_step_import_max_bytes(&view).unwrap(), 1);
        let view = environ(&[(STEP_IMPORT_MAX_BYTES_ENV, "536870912")]);
        assert_eq!(
            resolve_step_import_max_bytes(&view).unwrap(),
            MAX_STEP_IMPORT_BYTES
        );
    }

    #[test]
    fn a_malformed_value_refuses_with_the_workers_message() {
        for raw in ["abc", "1.0", "1MiB", "12 34", "0x10", "--1", "1_", "_1"] {
            let view = environ(&[(STEP_IMPORT_MAX_BYTES_ENV, raw)]);
            assert_eq!(
                resolve_step_import_max_bytes(&view).unwrap_err(),
                STEP_IMPORT_ENV_TYPE_MESSAGE,
                "{raw:?}"
            );
        }
    }

    #[test]
    fn an_out_of_range_value_refuses_with_the_workers_message() {
        for raw in ["0", "-1", "536870913", "99999999999999999999"] {
            let view = environ(&[(STEP_IMPORT_MAX_BYTES_ENV, raw)]);
            assert_eq!(
                resolve_step_import_max_bytes(&view).unwrap_err(),
                STEP_IMPORT_LIMIT_RANGE_MESSAGE,
                "{raw:?}"
            );
        }
    }

    #[test]
    fn the_python_integer_grammar_is_ported() {
        assert_eq!(parse_python_integer("0"), Some(0));
        assert_eq!(parse_python_integer("42"), Some(42));
        assert_eq!(parse_python_integer("+42"), Some(42));
        assert_eq!(parse_python_integer("-42"), Some(-42));
        assert_eq!(parse_python_integer("1_048_576"), Some(1_048_576));
        assert_eq!(parse_python_integer("1__0"), None);
        assert_eq!(parse_python_integer(""), None);
        assert_eq!(parse_python_integer("-"), None);
        assert_eq!(parse_python_integer(" 1"), None);
        assert_eq!(parse_python_integer("99999999999999999999"), None);
    }

    #[test]
    fn the_validation_bounds_match_the_worker() {
        assert_eq!(validate_step_import_max_bytes(1).unwrap(), 1);
        assert_eq!(
            validate_step_import_max_bytes(536_870_912).unwrap(),
            536_870_912
        );
        assert_eq!(
            validate_step_import_max_bytes(0).unwrap_err(),
            STEP_IMPORT_LIMIT_RANGE_MESSAGE
        );
        assert_eq!(
            validate_step_import_max_bytes(536_870_913).unwrap_err(),
            STEP_IMPORT_LIMIT_RANGE_MESSAGE
        );
    }

    /// The recorded answers of the Python worker's
    /// `resolve_step_import_max_bytes` for the same inputs.
    #[test]
    fn the_resolution_matches_the_python_worker_case_by_case() {
        type Answer = Result<usize, &'static str>;
        let cases: &[(&str, Answer)] = &[
            ("", Ok(67_108_864)),
            ("   ", Ok(67_108_864)),
            ("\t", Ok(67_108_864)),
            (" ", Ok(67_108_864)),
            ("0", Err(STEP_IMPORT_LIMIT_RANGE_MESSAGE)),
            ("1", Ok(1)),
            ("-1", Err(STEP_IMPORT_LIMIT_RANGE_MESSAGE)),
            ("+1", Ok(1)),
            ("67108864", Ok(67_108_864)),
            ("67108864 ", Ok(67_108_864)),
            (" 67108864 ", Ok(67_108_864)),
            ("1048576", Ok(1_048_576)),
            ("1_048_576", Ok(1_048_576)),
            ("536870912", Ok(536_870_912)),
            ("536870913", Err(STEP_IMPORT_LIMIT_RANGE_MESSAGE)),
            ("99999999999999999999", Err(STEP_IMPORT_LIMIT_RANGE_MESSAGE)),
            (
                "-99999999999999999999",
                Err(STEP_IMPORT_LIMIT_RANGE_MESSAGE),
            ),
            ("abc", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("1.0", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("1MiB", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("1e6", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("12 34", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("0x10", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("--1", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("+", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("-", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("1_", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("_1", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
            ("1__0", Err(STEP_IMPORT_ENV_TYPE_MESSAGE)),
        ];
        for (raw, expected) in cases {
            let view = environ(&[(STEP_IMPORT_MAX_BYTES_ENV, raw)]);
            let answer = resolve_step_import_max_bytes(&view);
            match expected {
                Ok(value) => assert_eq!(answer.unwrap(), *value, "{raw:?}"),
                Err(message) => assert_eq!(answer.unwrap_err(), *message, "{raw:?}"),
            }
        }
    }
}
