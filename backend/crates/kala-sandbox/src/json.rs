//! A byte-compatible encoder for the JSON the Python module produces.
//!
//! `kala_agent.tools.sandbox` writes JSON with `json.dumps(payload,
//! ensure_ascii=True)`. That encoding is *not* the canonical JSON of
//! `kala-json`: CPython keeps the insertion order of the dictionary and uses
//! the default `", "` / `": "` separators, while the canonical encoder sorts
//! keys and emits no insignificant whitespace. The probe protocol is a
//! line-oriented text protocol, so the exact CPython bytes are reproduced
//! here instead of the canonical form.
//!
//! The differential test compares these bytes with the live Python output.

/// Escape a string exactly like CPython's `json` encoder with
/// `ensure_ascii=True`.
///
/// Non-ASCII characters become `\uXXXX` escapes, and characters above the
/// basic multilingual plane become a UTF-16 surrogate pair.
pub fn escape_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{8}' => output.push_str("\\b"),
            '\u{c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                push_escape(&mut output, character as u32);
            }
            character if (character as u32) < 0x7f => output.push(character),
            character => {
                let code = character as u32;
                if code <= 0xffff {
                    push_escape(&mut output, code);
                } else {
                    let adjusted = code - 0x1_0000;
                    push_escape(&mut output, 0xd800 + (adjusted >> 10));
                    push_escape(&mut output, 0xdc00 + (adjusted & 0x3ff));
                }
            }
        }
    }
    output.push('"');
    output
}

fn push_escape(output: &mut String, code: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    output.push_str("\\u");
    for shift in [12, 8, 4, 0] {
        output.push(HEX[((code >> shift) & 0xf) as usize] as char);
    }
}

/// Render a float exactly like CPython's `repr`.
pub fn dump_float(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_positive() {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".to_owned()
        } else {
            "0.0".to_owned()
        };
    }
    // `{:e}` yields the shortest round-tripping scientific form, which carries
    // the digits and the decimal exponent needed to rebuild the CPython text.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("scientific notation always has an exponent");
    let exponent: i32 = exponent.parse().expect("exponent is an integer");
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    // The decimal point position: `decpt` digits precede the point.
    let decpt = exponent + 1;
    let mut output = String::new();
    if negative {
        output.push('-');
    }
    if !(-3..=16).contains(&decpt) {
        output.push_str(&digits[..1]);
        if digits.len() > 1 {
            output.push('.');
            output.push_str(&digits[1..]);
        }
        output.push('e');
        let exponent = decpt - 1;
        if exponent < 0 {
            output.push('-');
        } else {
            output.push('+');
        }
        let magnitude = exponent.unsigned_abs();
        if magnitude < 10 {
            output.push('0');
        }
        output.push_str(&magnitude.to_string());
    } else if decpt <= 0 {
        output.push_str("0.");
        for _ in 0..-decpt {
            output.push('0');
        }
        output.push_str(digits);
    } else if (decpt as usize) >= digits.len() {
        output.push_str(digits);
        for _ in 0..(decpt as usize - digits.len()) {
            output.push('0');
        }
        output.push_str(".0");
    } else {
        let split = decpt as usize;
        output.push_str(&digits[..split]);
        output.push('.');
        output.push_str(&digits[split..]);
    }
    output
}

/// One value of a Python-compatible JSON document.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    /// The JSON null literal.
    Null,
    /// A JSON boolean.
    Bool(bool),
    /// A JSON integer.
    Integer(i64),
    /// A JSON float.
    Float(f64),
    /// A JSON string.
    Text(String),
    /// A JSON array.
    Array(Vec<JsonValue>),
    /// A JSON object with insertion-ordered keys.
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// Build a JSON object from ordered pairs.
    pub fn object(pairs: Vec<(String, JsonValue)>) -> Self {
        Self::Object(pairs)
    }

    /// Render the value exactly like `json.dumps(value, ensure_ascii=True)`.
    pub fn dump(&self) -> String {
        let mut output = String::new();
        self.write(&mut output);
        output
    }

    fn write(&self, output: &mut String) {
        match self {
            Self::Null => output.push_str("null"),
            Self::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
            Self::Integer(value) => output.push_str(&value.to_string()),
            Self::Float(value) => output.push_str(&dump_float(*value)),
            Self::Text(value) => output.push_str(&escape_text(value)),
            Self::Array(values) => {
                output.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    value.write(output);
                }
                output.push(']');
            }
            Self::Object(pairs) => {
                output.push('{');
                for (index, (key, value)) in pairs.iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    output.push_str(&escape_text(key));
                    output.push_str(": ");
                    value.write(output);
                }
                output.push('}');
            }
        }
    }
}

/// Render a list of strings as a JSON array.
pub fn dump_string_list(values: &[String]) -> String {
    JsonValue::Array(
        values
            .iter()
            .map(|value| JsonValue::Text(value.clone()))
            .collect(),
    )
    .dump()
}

/// Round a float to three decimal places exactly like CPython's `round`.
///
/// CPython rounds the exact binary value of the double and breaks a tie
/// towards the even digit. Formatting the double with three fractional digits
/// applies the same rule to the exact value, so the result matches.
pub fn round_three(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let rendered = format!("{value:.3}");
    rendered.parse::<f64>().unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_like_cpython() {
        assert_eq!(escape_text("plain"), "\"plain\"");
        assert_eq!(escape_text("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(escape_text("line\nbreak"), "\"line\\nbreak\"");
        assert_eq!(escape_text("\u{7f}"), "\"\\u007f\"");
        assert_eq!(escape_text("\u{e9}"), "\"\\u00e9\"");
        assert_eq!(escape_text("\u{1f600}"), "\"\\ud83d\\ude00\"");
    }

    #[test]
    fn dumps_objects_in_insertion_order() {
        let value = JsonValue::object(vec![
            (
                "controls".to_owned(),
                JsonValue::Array(vec![JsonValue::Text("a".to_owned())]),
            ),
            ("detail".to_owned(), JsonValue::Text("x".to_owned())),
        ]);
        assert_eq!(value.dump(), "{\"controls\": [\"a\"], \"detail\": \"x\"}");
    }

    #[test]
    fn renders_floats_like_cpython() {
        assert_eq!(dump_float(0.0), "0.0");
        assert_eq!(dump_float(1.0), "1.0");
        assert_eq!(dump_float(1.235), "1.235");
        assert_eq!(dump_float(183.679), "183.679");
        assert_eq!(dump_float(1e16), "1e+16");
        assert_eq!(dump_float(1e15), "1000000000000000.0");
        assert_eq!(dump_float(0.0001), "0.0001");
        assert_eq!(dump_float(0.00001), "1e-05");
        assert_eq!(dump_float(-1.5), "-1.5");
    }

    #[test]
    fn rounds_like_cpython() {
        assert_eq!(round_three(1.23456), 1.235);
        assert_eq!(round_three(8.835), 8.835);
        assert_eq!(round_three(1.2345), 1.234);
        assert_eq!(round_three(0.0625), 0.062);
        assert_eq!(round_three(2.675), 2.675);
        assert_eq!(round_three(1.0005), 1.0);
        assert_eq!(round_three(-1.2345), -1.234);
    }
}
