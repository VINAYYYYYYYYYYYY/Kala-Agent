//! Byte-compatible JSON writing for the Studio HTTP surface.
//!
//! Studio JSON is compact and ordered. Most success bodies are Pydantic
//! `response_model` output: strings are UTF-8, insertion order is significant
//! and floats use pydantic-core's Rust float text (see
//! `geometry::preview_body`). Route-level and transport refusals use
//! `json.dumps(..., sort_keys=True)` with `ensure_ascii=True`; validation
//! errors use the ordered `code`/`message`/`details` model with UTF-8 text.
//! This module exposes both string modes and never applies canonical-JSON
//! limits to a Studio response; preview meshes are legitimately larger than
//! the tool protocol's bounds.

use kala_json::format_float;

/// One JSON value for a response body.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i128),
    UInt(u64),
    Float(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
    /// Pre-rendered JSON text, embedded verbatim.
    ///
    /// The worker owns the design document's exact serialization, so the
    /// front door embeds the worker's own bytes instead of re-encoding them.
    /// The text is always a JSON document the worker produced.
    Raw(String),
}

/// A response value could not be rendered (non-finite float or non-UTF-8
/// JSON text). Studio responses are constructed from validated values, so a
/// failure here is an internal error rather than a client error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonWriteError;

impl Json {
    /// Build an object from borrowed keys.
    #[must_use]
    pub fn object(entries: Vec<(&'static str, Json)>) -> Self {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        )
    }

    /// Render ordered, non-ASCII-preserving JSON.
    ///
    /// # Errors
    ///
    /// Returns [`JsonWriteError`] for a non-finite float. All strings can be
    /// represented because Rust strings are valid UTF-8.
    pub fn to_bytes(&self) -> Result<Vec<u8>, JsonWriteError> {
        let mut out = String::new();
        self.encode(&mut out, false, false)?;
        Ok(out.into_bytes())
    }

    /// Render sorted-key ASCII JSON used by the refusal paths.
    ///
    /// # Errors
    ///
    /// Returns [`JsonWriteError`] for a non-finite float.
    pub fn to_sorted_ascii_bytes(&self) -> Result<Vec<u8>, JsonWriteError> {
        let mut out = String::new();
        self.encode(&mut out, true, true)?;
        Ok(out.into_bytes())
    }

    /// Render `json.dumps(..., indent=2, sort_keys=True)` ASCII bytes.
    ///
    /// This is the settings-file format; it is not used for HTTP bodies.
    ///
    /// # Errors
    ///
    /// Returns [`JsonWriteError`] for a non-finite float.
    pub fn to_pretty_sorted_ascii(&self) -> Result<String, JsonWriteError> {
        let mut out = String::new();
        self.encode_pretty(&mut out, 0, true)?;
        Ok(out)
    }

    fn encode_pretty(
        &self,
        out: &mut String,
        depth: usize,
        sort_keys: bool,
    ) -> Result<(), JsonWriteError> {
        match self {
            Self::Array(values) if !values.is_empty() => {
                out.push_str("[\n");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push_str(",\n");
                    }
                    push_indent(out, depth + 1);
                    value.encode_pretty(out, depth + 1, sort_keys)?;
                }
                out.push('\n');
                push_indent(out, depth);
                out.push(']');
            }
            Self::Object(entries) if !entries.is_empty() => {
                out.push_str("{\n");
                let mut ordered: Vec<&(String, Json)> = entries.iter().collect();
                if sort_keys {
                    ordered.sort_by(|left, right| left.0.cmp(&right.0));
                }
                for (index, (key, value)) in ordered.iter().enumerate() {
                    if index > 0 {
                        out.push_str(",\n");
                    }
                    push_indent(out, depth + 1);
                    encode_text(key, out, true);
                    out.push_str(": ");
                    value.encode_pretty(out, depth + 1, sort_keys)?;
                }
                out.push('\n');
                push_indent(out, depth);
                out.push('}');
            }
            _ => {
                let mut compact = String::new();
                self.encode(&mut compact, true, sort_keys)?;
                out.push_str(&compact);
            }
        }
        Ok(())
    }

    fn encode(
        &self,
        out: &mut String,
        ensure_ascii: bool,
        sort_keys: bool,
    ) -> Result<(), JsonWriteError> {
        match self {
            Self::Raw(text) => out.push_str(text),
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Int(value) => out.push_str(&value.to_string()),
            Self::UInt(value) => out.push_str(&value.to_string()),
            Self::Float(value) => {
                let rendered = format_float(*value).map_err(|_| JsonWriteError)?;
                out.push_str(&rendered);
            }
            Self::String(value) => encode_text(value, out, ensure_ascii),
            Self::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    value.encode(out, ensure_ascii, sort_keys)?;
                }
                out.push(']');
            }
            Self::Object(entries) => {
                out.push('{');
                if sort_keys {
                    let mut ordered: Vec<&(String, Json)> = entries.iter().collect();
                    ordered.sort_by(|left, right| left.0.cmp(&right.0));
                    for (index, (key, value)) in ordered.iter().enumerate() {
                        if index > 0 {
                            out.push(',');
                        }
                        encode_text(key, out, ensure_ascii);
                        out.push(':');
                        value.encode(out, ensure_ascii, sort_keys)?;
                    }
                } else {
                    for (index, (key, value)) in entries.iter().enumerate() {
                        if index > 0 {
                            out.push(',');
                        }
                        encode_text(key, out, ensure_ascii);
                        out.push(':');
                        value.encode(out, ensure_ascii, sort_keys)?;
                    }
                }
                out.push('}');
            }
        }
        Ok(())
    }
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Quote a string exactly as `json.dumps(..., ensure_ascii=True)` does.
#[must_use]
pub fn quote_ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    encode_text(text, &mut out, true);
    out
}

/// CPython `json.dumps` string escaping in either ASCII or UTF-8 mode.
fn encode_text(text: &str, out: &mut String, ensure_ascii: bool) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{0c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character if ensure_ascii && !character.is_ascii() => {
                let point = character as u32;
                if point <= 0xFFFF {
                    out.push_str(&format!("\\u{point:04x}"));
                } else {
                    let value = point - 0x1_0000;
                    let high = 0xD800 + (value >> 10);
                    let low = 0xDC00 + (value & 0x3FF);
                    out.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                }
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn ordered_float_and_utf8_output_matches_cpython() {
        let value = Json::Object(vec![
            ("z".into(), Json::Float(1e-5)),
            ("a".into(), Json::Float(1.0)),
            ("u".into(), Json::String("bøks".into())),
            ("n".into(), Json::Null),
        ]);
        assert_eq!(
            String::from_utf8(value.to_bytes().unwrap()).unwrap(),
            "{\"z\":1e-05,\"a\":1.0,\"u\":\"bøks\",\"n\":null}"
        );
    }

    #[test]
    fn sorted_ascii_output_matches_cpython_error_path() {
        let value = Json::Object(vec![
            ("message".into(), Json::String("é".into())),
            ("details".into(), Json::Array(vec![])),
            ("code".into(), Json::String("x".into())),
        ]);
        assert_eq!(
            String::from_utf8(value.to_sorted_ascii_bytes().unwrap()).unwrap(),
            "{\"code\":\"x\",\"details\":[],\"message\":\"\\u00e9\"}"
        );
    }

    #[test]
    fn negative_zero_and_exponents_are_cpython_repr() {
        assert_eq!(
            String::from_utf8(
                Json::Array(vec![Json::Float(-0.0), Json::Float(1e16)])
                    .to_bytes()
                    .unwrap()
            )
            .unwrap(),
            "[-0.0,1e+16]"
        );
    }
}
