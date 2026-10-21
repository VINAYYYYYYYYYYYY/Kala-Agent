use crate::error::CanonicalJsonError;

/// The bounded JSON protocol limits from `kala_agent.tools.protocol`.
///
/// `max_json_bytes` applies to the raw input in [`crate::parse_strict`] and to
/// the canonical ASCII output in [`crate::canonical_dumps`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum raw input bytes or canonical output bytes.
    pub max_json_bytes: usize,
    /// Maximum string length in Unicode code points (Python `len`).
    pub max_string_length: usize,
    /// Maximum array length.
    pub max_collection_items: usize,
    /// Maximum object key length in Unicode code points.
    pub max_key_length: usize,
    /// Maximum nesting depth. The root value is depth zero.
    ///
    /// `usize::MAX` disables the depth check. The parser, validator, encoder
    /// and value lifecycle are iterative, so an unbounded domain does not rely
    /// on the thread stack.
    pub max_json_depth: usize,
    /// Maximum decimal digits in an integer literal. CPython 3.11+ limits
    /// `int(str)` conversions to 4300 digits by default; `0` disables the
    /// check for callers that explicitly change that process setting.
    pub max_integer_digits: usize,
}

impl Limits {
    /// Python's `MAX_JSON_DEPTH`.
    pub const DEFAULT_MAX_JSON_DEPTH: usize = 48;
    /// Python's `MAX_JSON_BYTES`.
    pub const DEFAULT_MAX_JSON_BYTES: usize = 1_048_576;
    /// Python's `MAX_STRING_LENGTH`.
    ///
    /// The tool protocol passes a design's compiled FreeCAD call list as one
    /// JSON string; a 500-step design is up to 278 514 characters, so this
    /// bound is 512 KiB, exactly like the Python constant.
    pub const DEFAULT_MAX_STRING_LENGTH: usize = 524_288;
    /// Python's `MAX_COLLECTION_ITEMS`.
    pub const DEFAULT_MAX_COLLECTION_ITEMS: usize = 2_048;
    /// Python's `MAX_KEY_LENGTH`.
    pub const DEFAULT_MAX_KEY_LENGTH: usize = 256;
    /// CPython's default `sys.int_info.default_max_str_digits`.
    pub const DEFAULT_MAX_INTEGER_DIGITS: usize = 4_300;

    /// The production protocol limits.
    pub const fn protocol() -> Self {
        Self {
            max_json_bytes: Self::DEFAULT_MAX_JSON_BYTES,
            max_string_length: Self::DEFAULT_MAX_STRING_LENGTH,
            max_collection_items: Self::DEFAULT_MAX_COLLECTION_ITEMS,
            max_key_length: Self::DEFAULT_MAX_KEY_LENGTH,
            max_json_depth: Self::DEFAULT_MAX_JSON_DEPTH,
            max_integer_digits: Self::DEFAULT_MAX_INTEGER_DIGITS,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::protocol()
    }
}

pub(crate) fn check_depth(depth: usize, limits: &Limits) -> Result<(), CanonicalJsonError> {
    if depth > limits.max_json_depth {
        Err(CanonicalJsonError::DepthExceeded)
    } else {
        Ok(())
    }
}
