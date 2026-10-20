//! Python compatible text rendering.
//!
//! Two Python behaviours are reproduced here:
//!
//! * `str.strip()` removes the characters CPython treats as whitespace, not
//!   the Unicode whitespace set of Rust's `str::trim`.
//! * `repr()` of a string, which appears verbatim in the schema validation
//!   messages (`required property 'a' is not declared`).

use kala_json::{CanonicalValue, JsonText};

/// Whether a code point is whitespace for CPython's `str.strip()`.
///
/// CPython strips every character whose `str.isspace()` is true: the ASCII
/// whitespace, the C1 control characters `0x1c` to `0x1f` and the Unicode
/// space separators.
pub(crate) fn is_python_whitespace(code_point: u32) -> bool {
    matches!(
        code_point,
        0x09..=0x0D
            | 0x1C..=0x1F
            | 0x20
            | 0x85
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

/// Strip CPython's whitespace from both ends of a text.
pub(crate) fn python_strip(text: &JsonText) -> JsonText {
    let code_points = text.code_points();
    let mut start = 0usize;
    let mut end = code_points.len();
    while start < end && is_python_whitespace(code_points[start]) {
        start += 1;
    }
    while end > start && is_python_whitespace(code_points[end - 1]) {
        end -= 1;
    }
    JsonText::from_code_points(code_points[start..end].to_vec()).expect("valid code points")
}

/// Whether a code point is printable in CPython's `repr()`.
///
/// CPython prints a character as itself when it is printable, which excludes
/// the Unicode "Other" and "Separator" categories, apart from the ASCII
/// space.
fn is_python_printable(code_point: u32) -> bool {
    let Some(character) = char::from_u32(code_point) else {
        return false;
    };
    if character == ' ' {
        return true;
    }
    if character.is_control() {
        return false;
    }
    !matches!(
        character,
        '\u{a0}'
            | '\u{1680}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    ) && !('\u{2000}'..='\u{200a}').contains(&character)
}

/// The CPython `repr()` of a string.
pub(crate) fn python_repr_text(text: &JsonText) -> String {
    let code_points = text.code_points();
    let has_single = code_points.contains(&u32::from(b'\''));
    let has_double = code_points.contains(&u32::from(b'"'));
    let quote = if has_single && !has_double { '"' } else { '\'' };
    let mut out = String::with_capacity(code_points.len() + 2);
    out.push(quote);
    for code_point in code_points {
        match code_point {
            0x5C => out.push_str("\\\\"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            value if *value == u32::from(b'\'') || *value == u32::from(b'"') => {
                out.push('\\');
                out.push(char::from_u32(*value).expect("ASCII"));
            }
            value if is_python_printable(*value) => {
                out.push(char::from_u32(*value).expect("printable"));
            }
            value if *value < 0x100 => {
                out.push_str(&format!("\\x{value:02x}"));
            }
            value if *value < 0x1_0000 => {
                out.push_str(&format!("\\u{value:04x}"));
            }
            value => {
                out.push_str(&format!("\\U{value:08x}"));
            }
        }
    }
    out.push(quote);
    out
}

/// The CPython `repr()` of a JSON value.
pub(crate) fn python_repr(value: &CanonicalValue) -> String {
    match value {
        CanonicalValue::Text(text) => python_repr_text(text),
        CanonicalValue::Bool(true) => "True".to_owned(),
        CanonicalValue::Bool(false) => "False".to_owned(),
        CanonicalValue::Null => "None".to_owned(),
        CanonicalValue::Integer(number) => number.to_string(),
        CanonicalValue::Float(number) => format_float_repr(*number),
        CanonicalValue::Array(items) => {
            let rendered: Vec<String> = items.iter().map(python_repr).collect();
            format!("[{}]", rendered.join(", "))
        }
        CanonicalValue::Object(entries) => {
            let rendered: Vec<String> = entries
                .iter()
                .map(|(key, value)| format!("{}: {}", python_repr_text(key), python_repr(value)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

/// The CPython `repr()` of a finite float.
fn format_float_repr(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e16 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}
