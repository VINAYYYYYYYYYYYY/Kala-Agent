//! The wire form of the answers of the API.
//!
//! The Python source writes the answers of its routes with the JSON
//! serializer of Pydantic, which is written in Rust, and not with the JSON
//! serializer of CPython. The two write the same bytes for everything but the
//! floating point numbers: the Rust serializer writes a number in the fixed
//! form whenever the fixed form is the shorter one, and CPython writes the
//! scientific form. `2.36e-05` is `0.0000236` in the answer of the live API
//! and `2.36e-05` in the answer of `json.dumps`.
//!
//! This module writes the answers the way the live API writes them. The
//! floating point numbers are written by the accepted `serde_json` crate,
//! whose number format is the one of the Rust serializer of Pydantic, and the
//! strings are written with the escaping of CPython, which the two
//! serializers share: the quote, the backslash and the control characters are
//! escaped, and every other character is written as it is.

use kala_contracts::{ContractError, Text, WireValue};

/// Write one answer as the bytes the live API writes.
///
/// # Errors
///
/// Returns the reason the document cannot be written, which happens when a
/// text carries a lone surrogate: the UTF-8 form of the answer cannot hold
/// it, exactly as the UTF-8 encoding of CPython cannot.
pub fn encode_answer(value: &WireValue) -> Result<String, ContractError> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

/// Write one value.
fn write_value(value: &WireValue, out: &mut String) -> Result<(), ContractError> {
    match value {
        WireValue::Null => out.push_str("null"),
        WireValue::Bool(true) => out.push_str("true"),
        WireValue::Bool(false) => out.push_str("false"),
        WireValue::Integer(number) => {
            let rendered = number.to_string();
            out.push_str(&rendered);
        }
        WireValue::Float(number) => {
            let rendered = serde_json::to_string(number)
                .map_err(|error| ContractError::new("WireValue", error.to_string()))?;
            out.push_str(&rendered);
        }
        WireValue::Text(text) => write_text(text, out)?,
        WireValue::Array(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(value, out)?;
            }
            out.push(']');
        }
        WireValue::Object(fields) => {
            out.push('{');
            for (index, (name, value)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_text(name, out)?;
                out.push(':');
                write_value(value, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// Write one text with the escaping of the JSON serializers.
fn write_text(value: &Text, out: &mut String) -> Result<(), ContractError> {
    out.push('"');
    for point in value.code_points() {
        match *point {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x09 => out.push_str("\\t"),
            0x0A => out.push_str("\\n"),
            0x0C => out.push_str("\\f"),
            0x0D => out.push_str("\\r"),
            0x00..=0x1F => {
                out.push_str(&format!("\\u{point:04x}"));
            }
            0x20..=0x7E => out.push(char::from_u32(*point).expect("ASCII")),
            0x7F..=0x10_FFFF | 0x11_0000..=u32::MAX => match char::from_u32(*point) {
                Some(character) => out.push(character),
                None => {
                    return Err(ContractError::new(
                        "WireValue",
                        "text contains a lone surrogate and cannot be encoded as UTF-8",
                    ));
                }
            },
        }
    }
    out.push('"');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float(value: f64) -> WireValue {
        WireValue::Float(value)
    }

    #[test]
    fn a_float_is_written_the_way_the_live_api_writes_it() {
        assert_eq!(encode_answer(&float(2.36e-05)).unwrap(), "0.0000236");
        assert_eq!(encode_answer(&float(8.6e-06)).unwrap(), "8.6e-6");
        assert_eq!(encode_answer(&float(1e-05)).unwrap(), "0.00001");
        assert_eq!(encode_answer(&float(1e-06)).unwrap(), "1e-6");
        assert_eq!(encode_answer(&float(1e16)).unwrap(), "1e+16");
        assert_eq!(encode_answer(&float(120.0)).unwrap(), "120.0");
        assert_eq!(encode_answer(&float(0.0)).unwrap(), "0.0");
        assert_eq!(encode_answer(&float(-0.0)).unwrap(), "-0.0");
        assert_eq!(
            encode_answer(&float(0.3333333333333333)).unwrap(),
            "0.3333333333333333"
        );
        assert_eq!(
            encode_answer(&float(9543.451332235383)).unwrap(),
            "9543.451332235383"
        );
    }

    #[test]
    fn a_document_is_written_in_the_order_of_its_fields() {
        let document = WireValue::object([
            ("first", WireValue::Integer(1_u64.into())),
            ("second", WireValue::Text(kala_contracts::Text::new("two"))),
            ("third", WireValue::Array(vec![WireValue::Bool(true)])),
        ]);
        assert_eq!(
            encode_answer(&document).unwrap(),
            "{\"first\":1,\"second\":\"two\",\"third\":[true]}"
        );
    }

    #[test]
    fn a_text_is_escaped_the_way_the_json_serializers_escape_it() {
        let text = WireValue::Text(kala_contracts::Text::new("a\"b\\c\nd\u{0001}eé"));
        assert_eq!(
            encode_answer(&text).unwrap(),
            "\"a\\\"b\\\\c\\nd\\u0001eé\""
        );
    }
}
