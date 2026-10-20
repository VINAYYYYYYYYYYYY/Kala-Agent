//! Persisted canonical object domain and lossless numeric/text behaviour.

use kala_contracts::{CanonicalJsonObject, ContractModel, DesignSpec, model_json_limits};
use kala_json::{CanonicalValue, Limits, canonical_dumps, parse_strict};

fn spec_with(attributes: &str) -> DesignSpec {
    let source = format!(
        r#"{{"id":"s","name":"n","summary":"z","attributes_json":{}}}"#,
        json_string(attributes)
    );
    DesignSpec::from_json_str(&source).expect("valid spec")
}

fn json_string(value: &str) -> String {
    // Only used to build test input; the production crate never depends on
    // serde_json.
    let mut out = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[test]
fn persisted_objects_do_not_inherit_tool_limits() {
    let depth = 5_000;
    let deep = format!("{}1{}", "{\"a\":".repeat(depth), "}".repeat(depth));
    let spec = spec_with(&deep);
    let canonical = spec.attributes_json().as_str();
    assert_eq!(canonical.len(), deep.len());
    assert!(canonical.starts_with("{\"a\":{"));

    let large_value = "x".repeat(2_000_000);
    let large = format!(r#"{{"x":"{large_value}"}}"#);
    let spec = spec_with(&large);
    assert!(spec.attributes_json().as_str().len() > 2_000_000);
}

#[test]
fn arbitrary_integers_signed_zero_and_lone_surrogates_survive() {
    let source = r#"{"big":1180591620717411303424,"huge":999999999999999999999999999999999999999999,"zero":-0.0,"small":1e-05,"surrogate":"\ud800"}"#;
    let spec = spec_with(source);
    let canonical = spec.attributes_json().as_str();
    assert!(canonical.contains("1180591620717411303424"));
    assert!(canonical.contains("-0.0"));
    assert!(canonical.contains("1e-05"));
    assert!(canonical.contains("\\ud800"));
    let value = spec.attributes_json().to_value().unwrap();
    assert!(matches!(value, CanonicalValue::Object(_)));
}

#[test]
fn invalid_persisted_objects_are_refused() {
    for source in [
        "[]",
        "1",
        r#"{"a":1,"a":2}"#,
        r#"{"a":NaN}"#,
        r#"{"a":1e999}"#,
        r#"{"a":1} trailing"#,
    ] {
        let result = CanonicalJsonObject::parse(source);
        assert!(result.is_err(), "{source}");
    }
}

#[test]
fn canonical_object_itself_is_byte_stable() {
    let object = CanonicalJsonObject::parse(r#"{ "b": 2, "a": {"z": 1, "a": 2} }"#).unwrap();
    assert_eq!(object.as_str(), r#"{"a":{"a":2,"z":1},"b":2}"#);
    let value = parse_strict(object.as_str().as_bytes(), &Limits::default()).unwrap();
    assert_eq!(
        canonical_dumps(&value, &Limits::default()).unwrap(),
        object.as_str()
    );
}

#[test]
fn model_input_json_uses_last_wins_but_canonical_objects_stay_strict() {
    let source = r#"{"id":"s","id":"t","name":"n","summary":"z"}"#;
    assert_eq!(DesignSpec::from_json_str(source).unwrap().id(), "t");
    assert!(CanonicalJsonObject::parse(r#"{"a":1,"a":2}"#).is_err());
    assert!(parse_strict(source.as_bytes(), &model_json_limits()).is_err());
}
