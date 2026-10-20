//! Pydantic `response_model` float text must match the live oracle across
//! random finite bit patterns, including exponent padding and -0.0.

use serde_json::Value;

#[test]
fn serde_json_float_text_matches_pydantic() {
    let vectors: Value = serde_json::from_str(include_str!("data/float_vectors.json")).unwrap();
    let mut failures = Vec::new();
    for vector in vectors.as_array().unwrap() {
        let bits = u64::from_str_radix(vector["bits"].as_str().unwrap(), 16).unwrap();
        let value = f64::from_bits(bits);
        let actual = serde_json::to_string(&value).unwrap();
        if actual != vector["pydantic"].as_str().unwrap() {
            failures.push(format!(
                "bits={bits:016x} pydantic={} actual={actual} repr={}",
                vector["pydantic"], vector["repr"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
