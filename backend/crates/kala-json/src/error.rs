use thiserror::Error;

/// Failure while parsing, validating or serializing canonical JSON, or while
/// encoding chain fields.
///
/// The messages intentionally match the `JsonEnvelopeError` messages raised by
/// `kala_agent.tools.protocol` so callers can preserve the frozen error text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CanonicalJsonError {
    /// The input bytes are not valid UTF-8.
    #[error("payload is not valid UTF-8")]
    InvalidUtf8,

    /// The input is not strict JSON (syntax, trailing data, BOM, ...).
    #[error("payload is not strict JSON")]
    NotJson,

    /// A JSON object repeats a decoded key.
    #[error("duplicate JSON object keys are not permitted")]
    DuplicateKey,

    /// The legacy Python dump path accepts mappings with non-string keys only
    /// until serialization. The typed Rust value model cannot represent one.
    #[error("JSON object keys must be strings")]
    NonStringKey,

    /// Python's `parse_constant` refuses these tokens (and equivalent inputs
    /// supplied directly to the dump path).
    #[error("JSON constant {0} is not permitted")]
    NonFiniteConstant(&'static str),

    /// A float is NaN or infinite.
    #[error("JSON numbers must be finite")]
    NonFinite,

    /// Nesting exceeds [`crate::Limits::max_json_depth`].
    #[error("JSON nesting exceeds the supported depth")]
    DepthExceeded,

    /// A string exceeds [`crate::Limits::max_string_length`] code points.
    #[error("JSON string exceeds the bounded length")]
    StringTooLong,

    /// An object key exceeds [`crate::Limits::max_key_length`] code points.
    #[error("JSON object key exceeds the bounded length")]
    KeyTooLong,

    /// An array exceeds [`crate::Limits::max_collection_items`].
    #[error("JSON collection exceeds the bounded item count")]
    CollectionTooLarge,

    /// The raw input exceeds [`crate::Limits::max_json_bytes`].
    #[error("payload exceeds the bounded size")]
    InputTooLarge,

    /// The canonical ASCII output exceeds [`crate::Limits::max_json_bytes`].
    #[error("canonical JSON exceeds the bounded size")]
    OutputTooLarge,

    /// The Python dump path sees a value outside the JSON type domain (for
    /// example `Decimal`). The typed Rust value model makes this unreachable,
    /// but the variant keeps the compatibility vocabulary explicit.
    #[error("value is not JSON serializable")]
    NotSerializable,

    /// Python's `json.dumps` failed for another reason (for example the
    /// CPython integer-to-string digit limit).
    #[error("value cannot be canonicalized as JSON")]
    NotCanonicalizable,

    /// A chain string cannot be UTF-8 encoded because it contains a lone
    /// surrogate. Python raises `UnicodeEncodeError` in the same situation.
    #[error("chain field cannot be encoded as UTF-8")]
    InvalidChainText,

    /// A code point lies outside Unicode's `0..=0x10FFFF` range.
    #[error("text contains a code point outside Unicode")]
    InvalidCodePoint,

    /// A lone surrogate cannot be represented in a Rust `String`.
    #[error("text contains a lone surrogate and cannot be encoded as UTF-8")]
    LoneSurrogate,
}
