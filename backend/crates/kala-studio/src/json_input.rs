//! CPython `json.loads(bytes)`-compatible input decoding.
//!
//! `json.loads` on a `bytes` argument detects UTF-8, UTF-8-with-BOM,
//! UTF-16LE/BE (with or without a BOM) and UTF-32LE/BE (with or without a
//! BOM) before parsing. FastAPI passes the raw request body to that function,
//! so a Windows client that encodes JSON as UTF-16 is accepted by the Python
//! Studio. Native parsing must preserve that accepted domain without a
//! Python runtime dependency.
//!
//! The module also normalises CPython's non-standard `NaN` / `Infinity` /
//! `-Infinity` constants for serde while tracking the original character
//! position of every output character. Pydantic's `json_invalid` location is
//! a zero-based character offset in the decoded document, so an earlier
//! constant replacement must never shift a later malformed-input offset.

/// Why a JSON input could not be decoded or parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonInputError {
    /// The bytes are not a text encoding CPython's `json.loads` accepts.
    /// FastAPI answers 400 `request_rejected` for the resulting
    /// `UnicodeDecodeError`.
    Encoding,
    /// A lone UTF-16/UTF-32 surrogate or UTF-8 surrogate code point. CPython
    /// decodes it with `surrogatepass`; the native parser cannot represent a
    /// lone surrogate in a Rust string. The contained value is the character
    /// offset in the decoded document.
    Surrogate(usize),
    /// Well-formed text with malformed JSON. The contained value is the
    /// zero-based character offset CPython reports as `e.pos`.
    Syntax(usize),
}

/// Decoded text plus the normalised bytes handed to serde.
pub struct NormalisedJson {
    /// The document with CPython's non-standard constants replaced.
    pub bytes: Vec<u8>,
    /// For each output character, the character offset in the original
    /// decoded document. Empty when `tracked` is false.
    pub origins: Vec<usize>,
    /// Whether [`origins`](Self::origins) was built. When it was not, the
    /// normalised bytes are the original document, so offsets are the
    /// character count of the prefix.
    pub tracked: bool,
    /// Total character count of the original decoded document.
    pub original_chars: usize,
}

/// Decode `bytes` with CPython's `json.detect_encoding` rules.
///
/// # Errors
///
/// Returns [`JsonInputError::Encoding`] for undecodable input and
/// [`JsonInputError::Surrogate`] for a lone surrogate code point.
pub fn decode_json_bytes(bytes: &[u8]) -> Result<String, JsonInputError> {
    const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
    const UTF16_BE_BOM: [u8; 2] = [0xFE, 0xFF];
    const UTF16_LE_BOM: [u8; 2] = [0xFF, 0xFE];
    const UTF32_BE_BOM: [u8; 4] = [0x00, 0x00, 0xFE, 0xFF];
    const UTF32_LE_BOM: [u8; 4] = [0xFF, 0xFE, 0x00, 0x00];

    // CPython checks UTF-32 BOMs before the UTF-16 BOMs, because the
    // little-endian UTF-32 BOM begins with the little-endian UTF-16 BOM.
    if bytes.starts_with(&UTF32_BE_BOM) {
        return decode_utf32(&bytes[4..], Endian::Big);
    }
    if bytes.starts_with(&UTF32_LE_BOM) {
        return decode_utf32(&bytes[4..], Endian::Little);
    }
    if bytes.starts_with(&UTF16_BE_BOM) {
        return decode_utf16(&bytes[2..], Endian::Big);
    }
    if bytes.starts_with(&UTF16_LE_BOM) {
        return decode_utf16(&bytes[2..], Endian::Little);
    }
    if bytes.starts_with(&UTF8_BOM) {
        return decode_utf8(&bytes[3..]);
    }
    if bytes.len() >= 4 {
        // `json.detect_encoding` sniffs BOM-less UTF-16/UTF-32 from the
        // position of the first zero bytes. Valid JSON starts with ASCII
        // (whitespace, `{`, `[`, `"`, a digit, `-`, `t`, `f` or `n`), so the
        // heuristics are unambiguous for accepted documents.
        if bytes[0] == 0 {
            return if bytes[1] == 0 {
                decode_utf32(bytes, Endian::Big)
            } else {
                decode_utf16(bytes, Endian::Big)
            };
        }
        if bytes[1] == 0 {
            return if bytes[2] == 0 && bytes[3] == 0 {
                decode_utf32(bytes, Endian::Little)
            } else {
                decode_utf16(bytes, Endian::Little)
            };
        }
    }
    decode_utf8(bytes)
}

#[derive(Clone, Copy)]
enum Endian {
    Big,
    Little,
}

fn decode_utf8(bytes: &[u8]) -> Result<String, JsonInputError> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(error) => {
            let valid = error.valid_up_to();
            let remainder = &bytes[valid..];
            // Python decodes with `errors="surrogatepass"`, which accepts the
            // three-byte UTF-8 encoding of a surrogate code point. Rust
            // strings cannot hold it; report its character position.
            if remainder.len() >= 3
                && remainder[0] == 0xED
                && (0xA0..=0xBF).contains(&remainder[1])
                && (0x80..=0xBF).contains(&remainder[2])
            {
                return Err(JsonInputError::Surrogate(
                    std::str::from_utf8(&bytes[..valid]).map_or(0, |prefix| prefix.chars().count()),
                ));
            }
            Err(JsonInputError::Encoding)
        }
    }
}

fn decode_utf16(bytes: &[u8], endian: Endian) -> Result<String, JsonInputError> {
    if bytes.len() % 2 != 0 {
        return Err(JsonInputError::Encoding);
    }
    let unit = |chunk: &[u8]| -> u16 {
        match endian {
            Endian::Big => u16::from_be_bytes([chunk[0], chunk[1]]),
            Endian::Little => u16::from_le_bytes([chunk[0], chunk[1]]),
        }
    };
    let mut text = String::with_capacity(bytes.len() / 2);
    let mut characters = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        let first = unit(&bytes[index..index + 2]);
        index += 2;
        if (0xD800..=0xDBFF).contains(&first) {
            if index < bytes.len() {
                let second = unit(&bytes[index..index + 2]);
                if (0xDC00..=0xDFFF).contains(&second) {
                    index += 2;
                    let combined = 0x1_0000
                        + ((u32::from(first) - 0xD800) << 10)
                        + (u32::from(second) - 0xDC00);
                    if let Some(character) = char::from_u32(combined) {
                        text.push(character);
                        characters += 1;
                        continue;
                    }
                }
            }
            return Err(JsonInputError::Surrogate(characters));
        }
        if (0xDC00..=0xDFFF).contains(&first) {
            return Err(JsonInputError::Surrogate(characters));
        }
        if let Some(character) = char::from_u32(u32::from(first)) {
            text.push(character);
            characters += 1;
        } else {
            return Err(JsonInputError::Encoding);
        }
    }
    Ok(text)
}

fn decode_utf32(bytes: &[u8], endian: Endian) -> Result<String, JsonInputError> {
    if bytes.len() % 4 != 0 {
        return Err(JsonInputError::Encoding);
    }
    let mut text = String::with_capacity(bytes.len() / 4);
    for (characters, chunk) in bytes.chunks_exact(4).enumerate() {
        let value = match endian {
            Endian::Big => u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
            Endian::Little => u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
        };
        if (0xD800..=0xDFFF).contains(&value) {
            return Err(JsonInputError::Surrogate(characters));
        }
        let Some(character) = char::from_u32(value) else {
            return Err(JsonInputError::Encoding);
        };
        text.push(character);
    }
    Ok(text)
}

/// Replace CPython's accepted constants and remember original offsets.
///
/// # Errors
///
/// Returns the character offset just after a `NaN`/`Infinity`/`-Infinity`
/// token that is not followed by a JSON delimiter. CPython parses the
/// constant and then reports the following byte, so this is the source
/// `json_invalid` position; leaving the token for serde would report the
/// start of the token instead.
pub fn normalise_json(text: &str) -> Result<NormalisedJson, usize> {
    scan(text, true, Replacement::Constant)
}

/// Replace CPython's accepted constants without building the offset map.
///
/// Used by parsers whose errors carry no source position (provider replies,
/// the settings file).
///
/// # Errors
///
/// Returns the character offset just after a constant with no delimiter,
/// exactly like [`normalise_json`].
pub fn normalise_json_bytes(text: &str) -> Result<Vec<u8>, usize> {
    scan(text, false, Replacement::Constant).map(|normalised| normalised.bytes)
}

/// Replace every value the canonical parser cannot represent with `0`.
///
/// The settings store reads a document with Python `json.loads` semantics:
/// `NaN`/`Infinity` are accepted and an integer/float literal that overflows
/// to infinity is a legal float. Values other than the two provider strings
/// are ignored by the store, so this normaliser collapses those non-finite
/// values to `0` while leaving every string byte-for-byte untouched. It is
/// the fallback path used only when `serde_json` cannot parse the document
/// (currently, a Python-accepted lone-surrogate escape).
///
/// # Errors
///
/// Returns the character offset just after a constant with no delimiter,
/// exactly like [`normalise_json`].
pub fn normalise_python_values(text: &str) -> Result<Vec<u8>, usize> {
    scan(text, false, Replacement::Zero).map(|normalised| normalised.bytes)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Replacement {
    /// CPython's constants become equal-magnitude overflow literals serde
    /// accepts, preserving the original character offsets.
    Constant,
    /// Every non-finite value becomes `0` so a strict canonical parser can
    /// read the surrounding document.
    Zero,
}

fn scan(text: &str, track: bool, replacement: Replacement) -> Result<NormalisedJson, usize> {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut origins = if track {
        Vec::with_capacity(text.chars().count())
    } else {
        Vec::new()
    };
    let mut index = 0usize;
    let mut original = 0usize;
    let mut frames: Vec<Frame> = Vec::new();
    let mut root = Expect::Value;
    let mut in_string = false;
    let mut escaped = false;
    let mut string_role = StringRole::Value;
    let mut unknown = false;

    while index < bytes.len() {
        let Some(character) = text[index..].chars().next() else {
            break;
        };
        let width = character.len_utf8();

        if in_string {
            push_char(&mut output, &mut origins, track, character, original);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
                finish_string(&mut frames, &mut root, string_role, &mut unknown);
            }
            index += width;
            original += 1;
            continue;
        }

        match character {
            ' ' | '\t' | '\n' | '\r' => {
                push_char(&mut output, &mut origins, track, character, original);
                index += 1;
                original += 1;
            }
            '"' => {
                let expected = current_expect(&frames, root);
                string_role = match expected {
                    Expect::Value => StringRole::Value,
                    Expect::Key | Expect::KeyOrEnd => StringRole::Key,
                    Expect::Colon | Expect::AfterValue => {
                        unknown = true;
                        StringRole::Value
                    }
                };
                in_string = true;
                push_char(&mut output, &mut origins, track, character, original);
                index += 1;
                original += 1;
            }
            '{' => {
                if !unknown && current_expect(&frames, root) == Expect::Value {
                    frames.push(Frame {
                        is_object: true,
                        expect: Expect::KeyOrEnd,
                    });
                } else {
                    unknown = true;
                }
                push_char(&mut output, &mut origins, track, character, original);
                index += 1;
                original += 1;
            }
            '[' => {
                if !unknown && current_expect(&frames, root) == Expect::Value {
                    frames.push(Frame {
                        is_object: false,
                        expect: Expect::Value,
                    });
                } else {
                    unknown = true;
                }
                push_char(&mut output, &mut origins, track, character, original);
                index += 1;
                original += 1;
            }
            '}' => {
                if !unknown {
                    match frames.last_mut() {
                        Some(frame)
                            if frame.is_object
                                && matches!(
                                    frame.expect,
                                    Expect::KeyOrEnd | Expect::AfterValue
                                ) =>
                        {
                            frames.pop();
                            complete_value(&mut frames, &mut root);
                        }
                        _ => unknown = true,
                    }
                }
                push_char(&mut output, &mut origins, track, character, original);
                index += 1;
                original += 1;
            }
            ']' => {
                if !unknown {
                    match frames.last_mut() {
                        Some(frame)
                            if !frame.is_object
                                && matches!(frame.expect, Expect::Value | Expect::AfterValue) =>
                        {
                            frames.pop();
                            complete_value(&mut frames, &mut root);
                        }
                        _ => unknown = true,
                    }
                }
                push_char(&mut output, &mut origins, track, character, original);
                index += 1;
                original += 1;
            }
            ',' => {
                if !unknown {
                    match frames.last_mut() {
                        Some(frame) if frame.expect == Expect::AfterValue => {
                            frame.expect = if frame.is_object {
                                Expect::Key
                            } else {
                                Expect::Value
                            };
                        }
                        _ => unknown = true,
                    }
                }
                push_char(&mut output, &mut origins, track, character, original);
                index += 1;
                original += 1;
            }
            ':' => {
                if !unknown {
                    match frames.last_mut() {
                        Some(frame) if frame.is_object && frame.expect == Expect::Colon => {
                            frame.expect = Expect::Value;
                        }
                        _ => unknown = true,
                    }
                }
                push_char(&mut output, &mut origins, track, character, original);
                index += 1;
                original += 1;
            }
            _ => {
                if !unknown && current_expect(&frames, root) == Expect::Value {
                    if let Some((token, replacement_text)) = constant_at(bytes, index, replacement)
                    {
                        let next = bytes.get(index + token.len()).copied();
                        let delimiter = next.is_none_or(|byte| {
                            matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b',' | b']' | b'}')
                        });
                        if !delimiter {
                            return Err(original + token.len());
                        }
                        if track {
                            origins.extend(std::iter::repeat_n(original, replacement_text.len()));
                        }
                        output.extend_from_slice(replacement_text.as_bytes());
                        index += token.len();
                        original += token.len();
                        complete_value(&mut frames, &mut root);
                        continue;
                    }
                    // A number or `true`/`false`/`null` token.
                    let start = index;
                    while index < bytes.len() {
                        let byte = bytes[index];
                        if matches!(
                            byte,
                            b' ' | b'\t' | b'\n' | b'\r' | b',' | b']' | b'}' | b'{' | b'[' | b':'
                        ) || byte == b'"'
                        {
                            break;
                        }
                        let width = text[index..].chars().next().map_or(1, char::len_utf8);
                        index += width;
                        original += 1;
                    }
                    if index == start {
                        // A non-token byte at a value position (for example a
                        // stray `)`): copy it and let serde produce the error.
                        push_char(&mut output, &mut origins, track, character, original);
                        index += width;
                        original += 1;
                        unknown = true;
                    } else {
                        let token = &text[start..index];
                        if replacement == Replacement::Zero && is_non_finite_number(token) {
                            push_char(&mut output, &mut origins, track, '0', original);
                        } else {
                            for character in token.chars() {
                                push_char(&mut output, &mut origins, track, character, original);
                            }
                        }
                        complete_value(&mut frames, &mut root);
                    }
                } else {
                    if !unknown {
                        unknown = true;
                    }
                    push_char(&mut output, &mut origins, track, character, original);
                    index += width;
                    original += 1;
                }
            }
        }
    }

    Ok(NormalisedJson {
        bytes: output,
        origins,
        tracked: track,
        original_chars: original,
    })
}

fn constant_at(
    bytes: &[u8],
    index: usize,
    replacement: Replacement,
) -> Option<(&'static str, &'static str)> {
    let rest = &bytes[index..];
    let token = if rest.starts_with(b"-Infinity") {
        "-Infinity"
    } else if rest.starts_with(b"Infinity") {
        "Infinity"
    } else if rest.starts_with(b"NaN") {
        "NaN"
    } else {
        return None;
    };
    match replacement {
        Replacement::Constant => Some(match token {
            "-Infinity" => ("-Infinity", "-1e400"),
            "Infinity" => ("Infinity", "1e400"),
            _ => ("NaN", "1e400"),
        }),
        Replacement::Zero => Some(match token {
            "-Infinity" => ("-Infinity", "0"),
            "Infinity" => ("Infinity", "0"),
            _ => ("NaN", "0"),
        }),
    }
}

/// Whether a JSON number token overflows the finite `f64` domain.
///
/// CPython's `json.loads` accepts such a literal as `float('inf')`; a strict
/// canonical parser refuses it. Settings values other than the two provider
/// strings are ignored, so the fallback parser collapses them to `0`.
fn is_non_finite_number(token: &str) -> bool {
    token.parse::<f64>().is_ok_and(|value| !value.is_finite())
}

fn push_char(
    output: &mut Vec<u8>,
    origins: &mut Vec<usize>,
    track: bool,
    character: char,
    original: usize,
) {
    let mut buffer = [0u8; 4];
    output.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
    if track {
        origins.push(original);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    Value,
    Key,
    KeyOrEnd,
    Colon,
    AfterValue,
}

#[derive(Clone, Copy)]
enum StringRole {
    Key,
    Value,
}

struct Frame {
    is_object: bool,
    expect: Expect,
}

fn current_expect(frames: &[Frame], root: Expect) -> Expect {
    frames.last().map_or(root, |frame| frame.expect)
}

fn complete_value(frames: &mut [Frame], root: &mut Expect) {
    match frames.last_mut() {
        Some(frame) => frame.expect = Expect::AfterValue,
        None => *root = Expect::AfterValue,
    }
}

fn finish_string(frames: &mut [Frame], root: &mut Expect, role: StringRole, unknown: &mut bool) {
    match role {
        StringRole::Key => match frames.last_mut() {
            Some(frame)
                if frame.is_object && matches!(frame.expect, Expect::Key | Expect::KeyOrEnd) =>
            {
                frame.expect = Expect::Colon;
            }
            _ => *unknown = true,
        },
        StringRole::Value => {
            if current_expect(frames, *root) == Expect::Value {
                complete_value(frames, root);
            } else {
                *unknown = true;
            }
        }
    }
}

/// Placeholder used in place of a JSON lone-surrogate escape while serde
/// parses a provider response. U+FFFE is a Unicode noncharacter that a real
/// provider payload will not contain; the provider parser checks final
/// message fields for it to reproduce Pydantic's response validation.
pub const LONE_SURROGATE_SENTINEL: char = '\u{FFFE}';

/// Replace lone surrogate escapes with [`LONE_SURROGATE_SENTINEL`].
///
/// Returns `None` when the document has no lone surrogate escape, so callers
/// can borrow the original text. Valid surrogate pairs are preserved
/// unchanged, matching CPython's string value.
#[must_use]
pub fn substitute_lone_surrogates(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut output: Option<String> = None;
    let mut index = 0usize;
    let mut copied = 0usize;
    let mut in_string = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if !in_string {
            if byte == b'"' {
                in_string = true;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = false;
            index += 1;
            continue;
        }
        if byte != b'\\' {
            index += 1;
            continue;
        }
        let Some(escape) = bytes.get(index + 1).copied() else {
            break;
        };
        if escape != b'u' {
            index += 2;
            continue;
        }
        let Some(value) = hex4(bytes, index + 2) else {
            index += 2;
            continue;
        };
        if (0xD800..=0xDBFF).contains(&value) {
            let paired = bytes.get(index + 6) == Some(&b'\\')
                && bytes.get(index + 7) == Some(&b'u')
                && hex4(bytes, index + 8).is_some_and(|low| (0xDC00..=0xDFFF).contains(&low));
            if paired {
                index += 12;
                continue;
            }
        } else if !(0xDC00..=0xDFFF).contains(&value) {
            index += 6;
            continue;
        }
        // A lone surrogate escape: substitute and never return the escape.
        let mut buffer = output.take().unwrap_or_default();
        buffer.push_str(&text[copied..index]);
        buffer.push(LONE_SURROGATE_SENTINEL);
        copied = index + 6;
        index += 6;
        output = Some(buffer);
    }
    if let Some(mut buffer) = output {
        buffer.push_str(&text[copied..]);
        Some(buffer)
    } else {
        None
    }
}

fn hex4(bytes: &[u8], index: usize) -> Option<u32> {
    let slice = bytes.get(index..index + 4)?;
    let mut value = 0u32;
    for byte in slice {
        let digit = match byte {
            b'0'..=b'9' => u32::from(byte - b'0'),
            b'a'..=b'f' => u32::from(byte - b'a') + 10,
            b'A'..=b'F' => u32::from(byte - b'A') + 10,
            _ => return None,
        };
        value = value * 16 + digit;
    }
    Some(value)
}

/// Map serde's byte position in the normalised document back to the original
/// character offset Pydantic reports.
#[must_use]
pub fn original_offset(normalised: &NormalisedJson, normalised_byte_index: usize) -> usize {
    let index = normalised_byte_index.min(normalised.bytes.len());
    let characters = String::from_utf8_lossy(&normalised.bytes[..index])
        .chars()
        .count();
    if !normalised.tracked {
        return characters.min(normalised.original_chars);
    }
    normalised
        .origins
        .get(characters)
        .copied()
        .unwrap_or(normalised.original_chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str, little: bool, bom: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        if bom {
            bytes.extend_from_slice(if little { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for unit in text.encode_utf16() {
            let encoded = if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            };
            bytes.extend_from_slice(&encoded);
        }
        bytes
    }

    fn utf32(text: &str, little: bool, bom: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        if bom {
            bytes.extend_from_slice(if little {
                &[0xFF, 0xFE, 0x00, 0x00]
            } else {
                &[0x00, 0x00, 0xFE, 0xFF]
            });
        }
        for character in text.chars() {
            let value = character as u32;
            let encoded = if little {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            };
            bytes.extend_from_slice(&encoded);
        }
        bytes
    }

    fn utf16_unit(unit: u16, little: bool, bom: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        if bom {
            bytes.extend_from_slice(if little { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        let encoded = if little {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        };
        bytes.extend_from_slice(&encoded);
        bytes
    }

    #[test]
    fn detects_every_encoding_cpython_json_accepts() {
        let text = "{\"kind\":\"bøks 😀\"}";
        for bytes in [
            text.as_bytes().to_vec(),
            [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
            utf16(text, true, false),
            utf16(text, false, false),
            utf16(text, true, true),
            utf16(text, false, true),
            utf32(text, true, false),
            utf32(text, false, false),
            utf32(text, true, true),
            utf32(text, false, true),
        ] {
            assert_eq!(decode_json_bytes(&bytes).unwrap(), text);
        }
    }

    #[test]
    fn malformed_encodings_report_python_error_classes() {
        assert_eq!(
            decode_json_bytes(&[0xFF, 0xFE, 0x00]),
            Err(JsonInputError::Encoding)
        );
        assert_eq!(
            decode_json_bytes(&[0xFF, 0xFE, 0x00, 0x00, 0x01]),
            Err(JsonInputError::Encoding)
        );
        // U+110000 is outside the Unicode range.
        assert_eq!(
            decode_json_bytes(&[0x00, 0x00, 0xFE, 0xFF, 0x00, 0x11, 0x00, 0x00]),
            Err(JsonInputError::Encoding)
        );
        // A high surrogate with no low surrogate, and a lone low surrogate.
        assert_eq!(
            decode_json_bytes(&utf16_unit(0xD800, true, true)),
            Err(JsonInputError::Surrogate(0))
        );
        assert_eq!(
            decode_json_bytes(&utf16_unit(0xDC00, false, true)),
            Err(JsonInputError::Surrogate(0))
        );
        // Python's `surrogatepass` accepts the UTF-8 encoding of a surrogate.
        assert_eq!(
            decode_json_bytes(b"\xed\xa0\x80"),
            Err(JsonInputError::Surrogate(0))
        );
    }

    #[test]
    fn constants_keep_original_error_offsets() {
        let body = r#"{"kind":NaN,"parameters":}"#;
        let normalised = normalise_json(body).unwrap();
        assert_eq!(
            String::from_utf8(normalised.bytes.clone()).unwrap(),
            r#"{"kind":1e400,"parameters":}"#
        );
        let error = serde_json::from_slice::<serde_json::Value>(&normalised.bytes).unwrap_err();
        let line = error.line().max(1);
        let mut line_start = 0usize;
        for _ in 1..line {
            if let Some(offset) = normalised.bytes[line_start..]
                .iter()
                .position(|byte| *byte == b'\n')
            {
                line_start += offset + 1;
            }
        }
        let index = line_start + error.column().saturating_sub(1);
        assert_eq!(original_offset(&normalised, index), 25);
        assert_eq!(normalised.original_chars, body.chars().count());
    }

    #[test]
    fn invalid_constant_boundaries_report_pythons_position() {
        assert_eq!(normalise_json("NaN123").err(), Some(3));
        assert_eq!(normalise_json(r#"{"kind":NaN123}"#).err(), Some(11));
        assert_eq!(normalise_json("[NaN123]").err(), Some(4));
        assert_eq!(normalise_json(r#"{"kind":Infinity1}"#).err(), Some(16));
        assert_eq!(normalise_json(r#"{"kind":-Infinityx}"#).err(), Some(17));
        // A constant in a key or after a value keeps serde's own position;
        // it must not be rewritten.
        let normalised = normalise_json("{\"a\":1,NaN:2}").unwrap();
        assert_eq!(
            String::from_utf8(normalised.bytes).unwrap(),
            "{\"a\":1,NaN:2}"
        );
        // Constants inside strings are untouched.
        let normalised = normalise_json(r#"{"kind":"NaN123"}"#).unwrap();
        assert_eq!(
            String::from_utf8(normalised.bytes).unwrap(),
            r#"{"kind":"NaN123"}"#
        );
    }

    #[test]
    fn provider_shaped_constants_parse_as_numbers() {
        let body = br#"{"model":NaN,"choices":[{"message":{"content":"hi"}}],"usage":Infinity}"#;
        let normalised = normalise_json(std::str::from_utf8(body).unwrap()).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&normalised.bytes).unwrap();
        assert_eq!(value["choices"][0]["message"]["content"], "hi");
        assert!(value["model"].is_number());
    }

    #[test]
    fn lone_surrogate_substitution_preserves_pairs() {
        assert_eq!(substitute_lone_surrogates(r#""hi""#), None);
        assert_eq!(
            substitute_lone_surrogates(r#""\ud83d\ude00""#),
            None,
            "a valid pair is preserved"
        );
        let replaced = substitute_lone_surrogates(r#""\ud800""#).unwrap();
        assert!(replaced.contains(LONE_SURROGATE_SENTINEL));
        assert!(!replaced.contains("\\u"));
    }
}
