//! Live Python differential tests for the tool protocol layer.
//!
//! These tests are the only place Python is used. They spawn the repository
//! virtual environment and compare against the live product implementation of
//! `kala_agent.tools.protocol`, never against a copy of it.
//!
//! The corpus is meaningful and deterministic: curated cases for every
//! documented rule plus a randomised corpus generated from a fixed seed. The
//! corpus is canonicalised before it is sent to the oracle, so both sides see
//! object members in the same order and the error precedence is comparable.
//!
//! If no Python interpreter can be started the test fails rather than
//! silently skipping: this parity gate is required, not optional.

mod support;

use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::protocol::{
    MAX_JSON_BYTES, ToolRequestEnvelope, ToolResponseEnvelope, canonical_json_dumps,
    check_object_schema, resolve_workspace_path, sanitize_child_stderr, strict_json_loads,
    summarize_error, validate_against_schema,
};
use num_bigint::BigInt;
use support::{Lcg, assert_same, canonical, error_result, hex, ok_result, run_oracle};

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::text(value)
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::object(entries)
}

fn array(values: Vec<CanonicalValue>) -> CanonicalValue {
    CanonicalValue::Array(values)
}

fn integer(value: i64) -> CanonicalValue {
    CanonicalValue::Integer(BigInt::from(value))
}

fn cases(values: Vec<CanonicalValue>) -> CanonicalValue {
    CanonicalValue::Array(values)
}

// ---------------------------------------------------------------------------
// Strict decoding and canonical encoding.
// ---------------------------------------------------------------------------

fn parse_corpus() -> Vec<CanonicalValue> {
    let mut payloads: Vec<Vec<u8>> = vec![
        b"{}".to_vec(),
        b"[]".to_vec(),
        b"null".to_vec(),
        b"true".to_vec(),
        b"false".to_vec(),
        b"123".to_vec(),
        b"-123".to_vec(),
        b"1.5".to_vec(),
        b"1e3".to_vec(),
        b"0.1".to_vec(),
        b"-0.0".to_vec(),
        br#""text""#.to_vec(),
        br#"{"a":1,"b":[1,2,{"c":null}]}"#.to_vec(),
        br#"{"b":1,"a":2}"#.to_vec(),
        br#"{"\u00e9":"\u00e9"}"#.to_vec(),
        br#""\ud83d\ude00""#.to_vec(),
        br#""\ud800""#.to_vec(),
        br#""\u0000\u001f\u007f""#.to_vec(),
        b"1180591620717411303424".to_vec(),
        b"1".to_vec(),
        b"  {\"a\" : 1}  ".to_vec(),
        // Refused payloads.
        b"{not json".to_vec(),
        b"".to_vec(),
        b"[".to_vec(),
        b"{\"a\":1,\"a\":2}".to_vec(),
        b"{\"a\":1,\"a\":1}".to_vec(),
        b"NaN".to_vec(),
        b"Infinity".to_vec(),
        b"-Infinity".to_vec(),
        b"1e400".to_vec(),
        b"-1e400".to_vec(),
        b"{} {}".to_vec(),
        b"\xef\xbb\xbf{}".to_vec(),
        b"{\"a\":\"\xff\xfe\"}".to_vec(),
        b"[1,]".to_vec(),
        b"{'a': 1}".to_vec(),
        b"01".to_vec(),
        b"+1".to_vec(),
        b".5".to_vec(),
        b"1.".to_vec(),
        b"\"unterminated".to_vec(),
    ];
    // A payload that is too large for the byte budget.
    payloads.push(vec![b'x'; MAX_JSON_BYTES + 1]);
    // A payload that is exactly at the byte budget.
    let mut at_budget = b"\"".to_vec();
    at_budget.extend(std::iter::repeat_n(b'a', MAX_JSON_BYTES - 2));
    at_budget.push(b'"');
    payloads.push(at_budget);
    // A string longer than the bounded string length.
    let mut long_string = b"\"".to_vec();
    long_string.extend(std::iter::repeat_n(b'a', 32_769));
    long_string.push(b'"');
    payloads.push(long_string);
    // A key longer than the bounded key length.
    let mut long_key = b"{\"".to_vec();
    long_key.extend(std::iter::repeat_n(b'k', 257));
    long_key.extend(b"\":1}");
    payloads.push(long_key);
    // An array with more items than the bounded collection size.
    let mut long_array = b"[".to_vec();
    for index in 0..2_049 {
        if index > 0 {
            long_array.push(b',');
        }
        long_array.extend(b"0");
    }
    long_array.push(b']');
    payloads.push(long_array);
    // A document nested beyond the bounded depth.
    let mut deep = vec![b'['; 49];
    deep.push(b'1');
    deep.extend(std::iter::repeat_n(b']', 49));
    payloads.push(deep);
    // A document nested exactly at the bounded depth.
    let mut at_depth = vec![b'['; 48];
    at_depth.push(b'1');
    at_depth.extend(std::iter::repeat_n(b']', 48));
    payloads.push(at_depth);

    payloads
        .into_iter()
        .map(|payload| object(vec![("hex", text(&hex(&payload)))]))
        .collect()
}

fn parse_case(payload: &[u8]) -> CanonicalValue {
    match strict_json_loads(payload, MAX_JSON_BYTES) {
        Err(error) => error_result(&error),
        Ok(value) => match canonical_json_dumps(&value, MAX_JSON_BYTES) {
            Ok(encoded) => ok_result(text(&encoded)),
            Err(error) => error_result(&error),
        },
    }
}

#[test]
fn strict_decoding_and_canonical_encoding_match_live_python() {
    let corpus = parse_corpus();
    let oracle = run_oracle("parse", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let payload = hex_decode(case);
        let native = parse_case(&payload);
        assert_same(&native, &expected, "parse corpus case");
    }
}

fn hex_decode(case: &CanonicalValue) -> Vec<u8> {
    let CanonicalValue::Object(entry) = case else {
        panic!("case is not an object")
    };
    let CanonicalValue::Text(value) = entry.get(&JsonText::new("hex")).expect("hex") else {
        panic!("case has no hex")
    };
    let text = value.to_string_lossy();
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let characters: Vec<char> = text.chars().collect();
    for pair in characters.chunks(2) {
        let high = pair[0].to_digit(16).expect("hex digit") as u8;
        let low = pair[1].to_digit(16).expect("hex digit") as u8;
        bytes.push((high << 4) | low);
    }
    bytes
}

// ---------------------------------------------------------------------------
// Canonical encoding of values.
// ---------------------------------------------------------------------------

fn encode_corpus() -> Vec<CanonicalValue> {
    let mut values = vec![
        object(vec![]),
        array(vec![]),
        text(""),
        text("plain"),
        text("é"),
        text("😀"),
        integer(0),
        integer(-1),
        integer(1),
        CanonicalValue::Float(1.0),
        CanonicalValue::Float(-0.0),
        CanonicalValue::Float(0.1),
        CanonicalValue::Float(1e16),
        CanonicalValue::Float(1e-5),
        CanonicalValue::Bool(true),
        CanonicalValue::Bool(false),
        CanonicalValue::Null,
        object(vec![
            ("b", integer(1)),
            ("a", integer(2)),
            ("Z", integer(3)),
            ("z", integer(4)),
            ("é", integer(5)),
        ]),
        array(vec![
            integer(1),
            text("two"),
            CanonicalValue::Null,
            CanonicalValue::Bool(false),
            array(vec![integer(3)]),
            object(vec![("k", text("v"))]),
        ]),
        object(vec![(
            "big",
            CanonicalValue::Integer(BigInt::parse_bytes(b"1180591620717411303424", 10).unwrap()),
        )]),
        object(vec![(
            "nested",
            object(vec![("deep", array(vec![object(vec![("x", integer(1))])]))]),
        )]),
    ];
    // A randomised corpus of values.
    let mut random = Lcg::new(0x4B41_4C41_4A53_4F50);
    for _ in 0..120 {
        values.push(random_value(&mut random, 0));
    }
    values
        .into_iter()
        .map(|value| object(vec![("value", value)]))
        .collect()
}

fn random_value(random: &mut Lcg, depth: usize) -> CanonicalValue {
    let choice = random.below(10);
    match choice {
        0..=2 => {
            let length = random.below(8) as usize;
            let mut out = String::new();
            for _ in 0..length {
                let code_point = match random.below(5) {
                    0 => random.below(0x20) as u32,
                    1 => 0x20 + random.below(0x7F) as u32,
                    2 => 0x80 + random.below(0x700) as u32,
                    3 => 0xD800 + random.below(0x800) as u32,
                    _ => random.below(0x11_0000) as u32,
                };
                out.push(char::from_u32(code_point).unwrap_or('\u{FFFD}'));
            }
            text(&out)
        }
        3 => integer(random.next() as i64),
        4 => CanonicalValue::Float(f64::from_bits(random.next())),
        5 => CanonicalValue::Bool(random.below(2) == 0),
        6 => CanonicalValue::Null,
        7 if depth < 3 => {
            let length = random.below(4) as usize;
            array(
                (0..length)
                    .map(|_| random_value(random, depth + 1))
                    .collect(),
            )
        }
        8 if depth < 3 => {
            let length = random.below(4) as usize;
            let mut entries = CanonicalObject::new();
            for index in 0..length {
                let key = format!("k{index}");
                let value = random_value(random, depth + 1);
                entries.insert(JsonText::new(&key), value);
            }
            CanonicalValue::Object(entries)
        }
        _ => integer(0),
    }
}

#[test]
fn canonical_encoding_matches_live_python() {
    let corpus = encode_corpus();
    let oracle = run_oracle("encode", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entry) = case else {
            panic!("case is not an object")
        };
        let value = entry.get(&JsonText::new("value")).expect("value");
        let native = match canonical_json_dumps(value, MAX_JSON_BYTES) {
            Ok(encoded) => ok_result(text(&encoded)),
            Err(error) => error_result(&error),
        };
        assert_same(&native, &expected, "encode corpus case");
    }
}

// ---------------------------------------------------------------------------
// Schema validation.
// ---------------------------------------------------------------------------

fn validate_corpus() -> Vec<CanonicalValue> {
    let mut pairs: Vec<(CanonicalValue, CanonicalValue)> = Vec::new();
    let mut add = |value: CanonicalValue, schema: CanonicalValue| {
        pairs.push((value, schema));
    };

    // Types.
    for (value, kind) in [
        (integer(1), "integer"),
        (CanonicalValue::Float(1.5), "number"),
        (text("x"), "string"),
        (CanonicalValue::Bool(true), "boolean"),
        (CanonicalValue::Null, "null"),
        (array(vec![]), "array"),
        (object(vec![]), "object"),
    ] {
        add(value.clone(), object(vec![("type", text(kind))]));
        add(value.clone(), object(vec![("type", text(kind))]));
    }
    add(
        text("x"),
        object(vec![("type", array(vec![text("string"), text("null")]))]),
    );
    add(
        integer(1),
        object(vec![("type", array(vec![text("string"), text("null")]))]),
    );
    add(integer(1), object(vec![("type", text("date"))]));
    add(integer(1), object(vec![("type", array(vec![integer(7)]))]));
    add(integer(1), object(vec![("type", text("string"))]));
    add(
        integer(1),
        object(vec![("type", array(vec![text("integer"), text("null")]))]),
    );

    // Unsupported keywords.
    add(integer(1), object(vec![("nonsense", integer(1))]));
    add(
        integer(1),
        object(vec![("title", text("x")), ("type", text("integer"))]),
    );
    add(integer(1), object(vec![("$comment", text("x"))]));
    add(integer(1), object(vec![("format", text("x"))]));
    add(
        integer(1),
        object(vec![
            ("type", text("integer")),
            ("zzz", integer(1)),
            ("aaa", integer(2)),
        ]),
    );

    // const and enum.
    add(integer(1), object(vec![("const", integer(1))]));
    add(
        CanonicalValue::Bool(true),
        object(vec![("const", CanonicalValue::Bool(true))]),
    );
    add(
        integer(1),
        object(vec![("const", CanonicalValue::Bool(true))]),
    );
    add(integer(2), object(vec![("const", integer(1))]));
    add(
        integer(1),
        object(vec![("const", CanonicalValue::Float(1.0))]),
    );
    add(
        text("a"),
        object(vec![("enum", array(vec![text("a"), text("b")]))]),
    );
    add(text("a"), object(vec![("enum", array(vec![]))]));
    add(text("a"), object(vec![("enum", text("a"))]));
    add(
        text("c"),
        object(vec![("enum", array(vec![text("a"), text("b")]))]),
    );
    add(
        text("a"),
        object(vec![("enum", array(vec![text("a"), CanonicalValue::Null]))]),
    );

    // allOf, anyOf and oneOf.
    add(
        integer(2),
        object(vec![(
            "allOf",
            array(vec![
                object(vec![("type", text("integer"))]),
                object(vec![("minimum", integer(1))]),
            ]),
        )]),
    );
    add(
        integer(0),
        object(vec![(
            "allOf",
            array(vec![
                object(vec![("type", text("integer"))]),
                object(vec![("minimum", integer(1))]),
            ]),
        )]),
    );
    add(integer(1), object(vec![("allOf", array(vec![]))]));
    add(integer(1), object(vec![("allOf", text("nope"))]));
    add(
        integer(1),
        object(vec![(
            "anyOf",
            array(vec![
                object(vec![("type", text("string"))]),
                object(vec![("type", text("integer"))]),
            ]),
        )]),
    );
    add(
        CanonicalValue::Null,
        object(vec![(
            "anyOf",
            array(vec![object(vec![("type", text("string"))])]),
        )]),
    );
    add(
        integer(1),
        object(vec![(
            "oneOf",
            array(vec![
                object(vec![("type", text("integer"))]),
                object(vec![("minimum", integer(0))]),
            ]),
        )]),
    );
    add(
        text("x"),
        object(vec![(
            "oneOf",
            array(vec![object(vec![("type", text("string"))])]),
        )]),
    );
    add(integer(1), object(vec![("oneOf", array(vec![]))]));
    add(
        integer(1),
        object(vec![("anyOf", array(vec![text("nope")]))]),
    );

    // Objects.
    add(object(vec![]), object(vec![("properties", text("nope"))]));
    add(
        object(vec![]),
        object(vec![("properties", object(vec![("a", text("nope"))]))]),
    );
    add(object(vec![]), object(vec![("required", text("a"))]));
    add(
        object(vec![]),
        object(vec![("required", array(vec![integer(7)]))]),
    );
    add(
        object(vec![]),
        object(vec![("required", array(vec![text("a"), text("a")]))]),
    );
    add(
        object(vec![]),
        object(vec![
            ("required", array(vec![text("a")])),
            ("properties", object(vec![("b", object(vec![]))])),
        ]),
    );
    add(
        object(vec![]),
        object(vec![
            ("required", array(vec![text("a")])),
            ("properties", object(vec![("a", object(vec![]))])),
        ]),
    );
    add(object(vec![]), object(vec![("minProperties", integer(1))]));
    add(
        object(vec![("a", integer(1)), ("b", integer(2))]),
        object(vec![("maxProperties", integer(1))]),
    );
    add(
        object(vec![("a", integer(1)), ("z", integer(2))]),
        object(vec![("additionalProperties", CanonicalValue::Bool(false))]),
    );
    add(
        object(vec![("a", text("x"))]),
        object(vec![(
            "additionalProperties",
            object(vec![("type", text("string"))]),
        )]),
    );
    add(
        object(vec![("a", integer(1))]),
        object(vec![(
            "additionalProperties",
            object(vec![("type", text("string"))]),
        )]),
    );
    add(
        object(vec![]),
        object(vec![("additionalProperties", text("nope"))]),
    );
    add(
        object(vec![("a", integer(1))]),
        object(vec![
            ("type", text("object")),
            (
                "properties",
                object(vec![("a", object(vec![("type", text("integer"))]))]),
            ),
        ]),
    );
    add(
        object(vec![("a", text("x"))]),
        object(vec![
            ("type", text("object")),
            (
                "properties",
                object(vec![("a", object(vec![("type", text("integer"))]))]),
            ),
        ]),
    );

    // Arrays.
    add(
        array(vec![integer(0); 2_049]),
        object(vec![("type", text("array"))]),
    );
    add(
        array(vec![integer(1)]),
        object(vec![("minItems", integer(2))]),
    );
    add(
        array(vec![integer(1), integer(2)]),
        object(vec![("maxItems", integer(1))]),
    );
    add(
        array(vec![integer(1)]),
        object(vec![("items", text("nope"))]),
    );
    add(
        array(vec![integer(1), text("x")]),
        object(vec![("items", object(vec![("type", text("integer"))]))]),
    );
    add(
        array(vec![integer(1), integer(2)]),
        object(vec![("uniqueItems", CanonicalValue::Bool(true))]),
    );
    add(
        array(vec![
            object(vec![("a", integer(1))]),
            object(vec![("a", integer(1))]),
        ]),
        object(vec![("uniqueItems", CanonicalValue::Bool(true))]),
    );
    add(
        array(vec![
            object(vec![("a", integer(1))]),
            object(vec![("a", integer(2))]),
        ]),
        object(vec![("uniqueItems", CanonicalValue::Bool(true))]),
    );

    // Strings.
    add(text("ab"), object(vec![("minLength", integer(3))]));
    add(text("abcd"), object(vec![("maxLength", integer(3))]));
    add(
        text("abc"),
        object(vec![
            ("pattern", text("^[a-z]+$")),
            ("maxLength", integer(8)),
        ]),
    );
    add(
        text("ABC"),
        object(vec![
            ("pattern", text("^[a-z]+$")),
            ("maxLength", integer(8)),
        ]),
    );
    add(text("abc"), object(vec![("pattern", integer(7))]));
    add(text("abc"), object(vec![("pattern", text("^a$"))]));

    // Numbers.
    add(
        integer(5),
        object(vec![("minimum", integer(0)), ("maximum", integer(10))]),
    );
    add(integer(-1), object(vec![("minimum", integer(0))]));
    add(integer(11), object(vec![("maximum", integer(10))]));
    add(integer(0), object(vec![("exclusiveMinimum", integer(0))]));
    add(integer(10), object(vec![("exclusiveMaximum", integer(10))]));
    add(integer(6), object(vec![("multipleOf", integer(3))]));
    add(integer(7), object(vec![("multipleOf", integer(3))]));
    add(integer(7), object(vec![("multipleOf", integer(0))]));
    add(integer(7), object(vec![("minimum", text("3"))]));
    add(
        integer(7),
        object(vec![("minimum", CanonicalValue::Bool(true))]),
    );
    add(
        CanonicalValue::Float(1.5),
        object(vec![("minimum", CanonicalValue::Float(1.5))]),
    );
    add(
        integer(1),
        object(vec![("minimum", CanonicalValue::Float(1.5))]),
    );
    add(
        integer(7),
        object(vec![("maximum", CanonicalValue::Float(1e308))]),
    );

    // A schema that is not an object.
    add(integer(1), array(vec![text("type")]));

    // A randomised corpus of schemas and values.
    let mut random = Lcg::new(0x5343_4845_4D41_5331);
    for _ in 0..200 {
        let schema = random_schema(&mut random, 0);
        let value = random_value(&mut random, 0);
        add(value, schema);
    }

    pairs
        .into_iter()
        .map(|(value, schema)| object(vec![("value", value), ("schema", schema)]))
        .collect()
}

fn random_schema(random: &mut Lcg, depth: usize) -> CanonicalValue {
    let mut entries = CanonicalObject::new();
    let choice = random.below(12);
    if depth > 2 {
        entries.insert(JsonText::new("type"), text("null"));
        return CanonicalValue::Object(entries);
    }
    match choice {
        0 => {
            entries.insert(JsonText::new("type"), text("string"));
            entries.insert(
                JsonText::new("maxLength"),
                integer(1 + random.below(16) as i64),
            );
            if random.below(2) == 0 {
                entries.insert(
                    JsonText::new("pattern"),
                    text(random.pick(&["^[a-z]+$", "^[a-z0-9-]+$", "^a$", "a|b"])),
                );
            }
        }
        1 => {
            entries.insert(JsonText::new("type"), text("integer"));
            if random.below(2) == 0 {
                entries.insert(JsonText::new("minimum"), integer(random.below(10) as i64));
            }
            if random.below(3) == 0 {
                entries.insert(JsonText::new("maximum"), integer(random.below(10) as i64));
            }
        }
        2 => {
            entries.insert(JsonText::new("type"), text("array"));
            if random.below(2) == 0 {
                entries.insert(JsonText::new("minItems"), integer(random.below(4) as i64));
            }
            if random.below(3) == 0 {
                entries.insert(JsonText::new("items"), random_schema(random, depth + 1));
            }
        }
        3 => {
            entries.insert(JsonText::new("type"), text("object"));
            let mut properties = CanonicalObject::new();
            let count = random.below(3) as usize;
            for index in 0..count {
                properties.insert(
                    JsonText::new(&format!("p{index}")),
                    random_schema(random, depth + 1),
                );
            }
            entries.insert(
                JsonText::new("properties"),
                CanonicalValue::Object(properties),
            );
            if random.below(2) == 0 {
                entries.insert(
                    JsonText::new("additionalProperties"),
                    CanonicalValue::Bool(random.below(2) == 0),
                );
            }
            if random.below(3) == 0 {
                entries.insert(JsonText::new("required"), array(vec![text("p0")]));
            }
        }
        4 => {
            entries.insert(JsonText::new("type"), text("number"));
            entries.insert(
                JsonText::new("multipleOf"),
                integer(1 + random.below(5) as i64),
            );
        }
        5 => {
            entries.insert(
                JsonText::new("enum"),
                array(vec![text("a"), text("b"), integer(1)]),
            );
        }
        6 => {
            entries.insert(JsonText::new("const"), integer(random.below(5) as i64));
        }
        7 => {
            entries.insert(
                JsonText::new("anyOf"),
                array(vec![
                    random_schema(random, depth + 1),
                    random_schema(random, depth + 1),
                ]),
            );
        }
        8 => {
            entries.insert(
                JsonText::new("oneOf"),
                array(vec![
                    random_schema(random, depth + 1),
                    random_schema(random, depth + 1),
                ]),
            );
        }
        9 => {
            entries.insert(
                JsonText::new("allOf"),
                array(vec![random_schema(random, depth + 1)]),
            );
        }
        10 => {
            entries.insert(JsonText::new("nonsense"), integer(random.below(3) as i64));
        }
        _ => {
            entries.insert(JsonText::new("type"), text("null"));
        }
    }
    CanonicalValue::Object(entries)
}

#[test]
fn schema_validation_matches_live_python() {
    let corpus = validate_corpus();
    let oracle = run_oracle("validate", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entry) = case else {
            panic!("case is not an object")
        };
        let value = entry.get(&JsonText::new("value")).expect("value");
        let schema = entry.get(&JsonText::new("schema")).expect("schema");
        let native = match validate_against_schema(value, schema, "$", 0) {
            Ok(()) => ok_result(CanonicalValue::Null),
            Err(error) => error_result(&error),
        };
        assert_same(&native, &expected, "validate corpus case");
    }
}

// ---------------------------------------------------------------------------
// Object schema admission.
// ---------------------------------------------------------------------------

fn check_corpus() -> Vec<CanonicalValue> {
    let schemas = vec![
        array(vec![text("type")]),
        object(vec![("type", text("string"))]),
        object(vec![("type", text("object"))]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(true)),
            ("properties", object(vec![("a", object(vec![]))])),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
            ("properties", object(vec![("a", text("nope"))])),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
            (
                "properties",
                object(vec![("a", object(vec![("nonsense", integer(1))]))]),
            ),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
            ("properties", object(vec![("a", object(vec![]))])),
            ("required", text("a")),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
            ("properties", object(vec![("a", object(vec![]))])),
            ("required", array(vec![text("z")])),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
            ("properties", object(vec![("a", object(vec![]))])),
            ("required", array(vec![integer(7)])),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
            ("properties", object(vec![("a", object(vec![]))])),
            ("pattern", text("^a$")),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
            (
                "properties",
                object(vec![("a", object(vec![("pattern", integer(7))]))]),
            ),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
            ("properties", object(vec![("a", object(vec![]))])),
        ]),
        object(vec![
            ("type", text("object")),
            ("additionalProperties", CanonicalValue::Bool(false)),
            (
                "properties",
                object(vec![(
                    "value",
                    object(vec![
                        ("maxLength", integer(32)),
                        ("pattern", text("^(a|aa)+$")),
                        ("type", text("string")),
                    ]),
                )]),
            ),
            ("required", array(vec![text("value")])),
        ]),
    ];
    schemas
        .into_iter()
        .map(|schema| object(vec![("schema", schema)]))
        .collect()
}

#[test]
fn object_schema_admission_matches_live_python() {
    let corpus = check_corpus();
    let oracle = run_oracle("check", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entry) = case else {
            panic!("case is not an object")
        };
        let schema = entry.get(&JsonText::new("schema")).expect("schema");
        let native = match check_object_schema(schema, "schema") {
            Ok(()) => ok_result(CanonicalValue::Null),
            Err(error) => error_result(&error),
        };
        assert_same(&native, &expected, "check corpus case");
    }
}

// ---------------------------------------------------------------------------
// Envelopes.
// ---------------------------------------------------------------------------

fn request_payloads() -> Vec<CanonicalValue> {
    let base = |overrides: Vec<(&str, CanonicalValue)>| {
        let mut entries = vec![
            ("protocol_version", text("1.0")),
            ("kind", text("request")),
            ("call_id", text("call-1")),
            ("tool_id", text("mechanical.reference-simulator")),
            ("tool_version", text("1.0.0")),
            (
                "arguments",
                object(vec![("operation", text("beam_volume"))]),
            ),
            ("workspace", text("/tmp/workspace")),
            ("deadline_seconds", CanonicalValue::Float(30.0)),
            ("max_output_bytes", integer(1_000_000)),
            ("max_artifact_bytes", integer(1_000_000)),
            ("granted_capabilities", array(vec![])),
        ];
        entries.extend(overrides);
        CanonicalValue::object(entries)
    };
    vec![
        base(vec![]),
        base(vec![(
            "granted_capabilities",
            array(vec![text("artifact.read")]),
        )]),
        base(vec![("arguments", object(vec![]))]),
        base(vec![("deadline_seconds", integer(30))]),
        base(vec![("max_output_bytes", integer(1))]),
        // Refused payloads.
        base(vec![("call_id", text("has space"))]),
        base(vec![("call_id", text(""))]),
        base(vec![("call_id", text("\u{a0}call-1\u{a0}"))]),
        base(vec![("call_id", text("\u{3000}call-1\u{3000}"))]),
        base(vec![("call_id", text("\u{1c}call-1\u{1c}"))]),
        base(vec![("call_id", text("\u{85}call-1\u{85}"))]),
        base(vec![("call_id", text("\tcall-1\t"))]),
        base(vec![("tool_id", integer(7))]),
        base(vec![("tool_version", text("has space"))]),
        base(vec![("protocol_version", text("has space"))]),
        base(vec![("arguments", array(vec![]))]),
        base(vec![("arguments", text("nope"))]),
        base(vec![("workspace", text("relative/workspace"))]),
        base(vec![("workspace", text(""))]),
        base(vec![("deadline_seconds", CanonicalValue::Float(0.0))]),
        base(vec![("deadline_seconds", text("30"))]),
        base(vec![("max_output_bytes", integer(0))]),
        base(vec![("max_artifact_bytes", integer(0))]),
        base(vec![("granted_capabilities", text("read"))]),
        base(vec![(
            "granted_capabilities",
            array(vec![text("has space")]),
        )]),
        base(vec![("kind", text("response"))]),
        object(vec![]),
        array(vec![]),
    ]
}

fn request_case(payload: &CanonicalValue) -> CanonicalValue {
    match ToolRequestEnvelope::from_payload(payload) {
        Err(error) => error_result(&error),
        Ok(envelope) => {
            let digest = match envelope.envelope_digest() {
                Ok(digest) => digest,
                Err(error) => return error_result(&error),
            };
            let json = match envelope.to_json() {
                Ok(json) => json,
                Err(error) => return error_result(&error),
            };
            ok_result(object(vec![
                ("arguments_digest", text(&envelope.arguments_digest())),
                ("arguments_json", text(envelope.arguments_json())),
                ("envelope_digest", text(&digest)),
                ("json", text(&json)),
                ("payload", envelope.to_payload()),
            ]))
        }
    }
}

/// A random member value for an envelope field.
fn random_field(random: &mut Lcg) -> CanonicalValue {
    match random.below(10) {
        0 => text("call-1"),
        1 => text("has space"),
        2 => text(""),
        3 => integer(random.next() as i64),
        4 => CanonicalValue::Float(random.next() as f64),
        5 => CanonicalValue::Bool(random.below(2) == 0),
        6 => CanonicalValue::Null,
        7 => array(vec![text("read")]),
        8 => object(vec![("a", integer(1))]),
        _ => text("1.0"),
    }
}

/// Random request payloads that exercise the validation order.
fn random_request_payloads() -> Vec<CanonicalValue> {
    let fields = [
        "protocol_version",
        "call_id",
        "tool_id",
        "tool_version",
        "arguments",
        "workspace",
        "deadline_seconds",
        "max_output_bytes",
        "max_artifact_bytes",
        "granted_capabilities",
    ];
    let mut random = Lcg::new(0x5245_5155_4553_5401);
    let mut payloads = Vec::with_capacity(200);
    for _ in 0..200 {
        let mut entries = CanonicalObject::new();
        for field in fields {
            if random.below(8) == 0 {
                continue;
            }
            entries.insert(JsonText::new(field), random_field(&mut random));
        }
        payloads.push(CanonicalValue::Object(entries));
    }
    payloads
}

/// Random response payloads that exercise the validation order.
fn random_response_payloads() -> Vec<CanonicalValue> {
    let fields = [
        "protocol_version",
        "call_id",
        "tool_id",
        "tool_version",
        "status",
        "result",
        "error_code",
        "error_message",
        "artifacts",
        "duration_ms",
    ];
    let mut random = Lcg::new(0x5245_5350_4F4E_5301);
    let mut payloads = Vec::with_capacity(200);
    for _ in 0..200 {
        let mut entries = CanonicalObject::new();
        for field in fields {
            if random.below(8) == 0 {
                continue;
            }
            let value = if field == "status" && random.below(2) == 0 {
                text(random.pick(&["succeeded", "failed", "denied", "timed_out", "elsewhere"]))
            } else {
                random_field(&mut random)
            };
            entries.insert(JsonText::new(field), value);
        }
        payloads.push(CanonicalValue::Object(entries));
    }
    payloads
}

#[test]
fn request_envelopes_match_live_python() {
    let corpus = request_payloads()
        .into_iter()
        .map(|payload| object(vec![("payload", payload)]))
        .collect::<Vec<_>>();
    let oracle = run_oracle("request", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entry) = case else {
            panic!("case is not an object")
        };
        let payload = entry.get(&JsonText::new("payload")).expect("payload");
        let native = request_case(payload);
        assert_same(&native, &expected, "request corpus case");
    }
}

fn response_payloads() -> Vec<CanonicalValue> {
    let artifact = object(vec![
        ("artifact_id", text("artifact-1")),
        ("uri", text("file:///tmp/report.json")),
        ("digest", text(&"a".repeat(64))),
        ("media_type", text("application/json")),
        ("size_bytes", integer(7)),
        ("classification", text("text")),
        ("preview_uri", CanonicalValue::Null),
        ("provenance_id", CanonicalValue::Null),
    ]);
    let base = |overrides: Vec<(&str, CanonicalValue)>| {
        let mut entries = vec![
            ("protocol_version", text("1.0")),
            ("kind", text("response")),
            ("call_id", text("call-1")),
            ("tool_id", text("mechanical.reference-simulator")),
            ("tool_version", text("1.0.0")),
            ("status", text("succeeded")),
            ("result", object(vec![("ok", CanonicalValue::Bool(true))])),
            ("error_code", CanonicalValue::Null),
            ("error_message", CanonicalValue::Null),
            ("artifacts", array(vec![])),
            ("duration_ms", CanonicalValue::Float(0.0)),
        ];
        entries.extend(overrides);
        CanonicalValue::object(entries)
    };
    vec![
        base(vec![]),
        base(vec![("artifacts", array(vec![artifact.clone()]))]),
        base(vec![("duration_ms", CanonicalValue::Float(1.5))]),
        base(vec![("result", CanonicalValue::Null)]),
        base(vec![
            ("status", text("failed")),
            ("result", CanonicalValue::Null),
            ("error_code", text("tool_error")),
            ("error_message", text("the tool failed")),
        ]),
        base(vec![
            ("status", text("denied")),
            ("result", CanonicalValue::Null),
            ("error_code", text("capability_denied")),
        ]),
        // Refused payloads.
        base(vec![("status", text("elsewhere"))]),
        base(vec![("result", array(vec![]))]),
        base(vec![("artifacts", text("nope"))]),
        base(vec![("artifacts", array(vec![text("nope")]))]),
        base(vec![(
            "artifacts",
            array(vec![object(vec![("artifact_id", text("a"))])]),
        )]),
        base(vec![
            ("status", text("failed")),
            ("result", object(vec![("ok", CanonicalValue::Bool(true))])),
            ("error_code", text("tool_error")),
        ]),
        base(vec![("error_code", text("has space"))]),
        base(vec![("error_message", text("   "))]),
        base(vec![("error_message", text(&"x".repeat(3_000)))]),
        base(vec![("duration_ms", CanonicalValue::Float(-1.0))]),
        base(vec![("duration_ms", text("1.0"))]),
        base(vec![("call_id", text("has space"))]),
        base(vec![("kind", text("request"))]),
        object(vec![]),
    ]
}

fn response_case(payload: &CanonicalValue) -> CanonicalValue {
    match ToolResponseEnvelope::from_payload(payload) {
        Err(error) => error_result(&error),
        Ok(envelope) => {
            let digest = match envelope.result_digest() {
                Ok(digest) => text(&digest),
                Err(error) => return error_result(&error),
            };
            let json = match envelope.to_json() {
                Ok(json) => text(&json),
                Err(error) => return error_result(&error),
            };
            let payload = match envelope.to_payload() {
                Ok(payload) => payload,
                Err(error) => return error_result(&error),
            };
            let result_json = match envelope.result_json() {
                Ok(None) => CanonicalValue::Null,
                Ok(Some(json)) => text(&json),
                Err(error) => return error_result(&error),
            };
            ok_result(object(vec![
                ("result_digest", digest),
                ("json", json),
                ("payload", payload),
                ("result_json", result_json),
                ("succeeded", CanonicalValue::Bool(envelope.succeeded())),
            ]))
        }
    }
}

#[test]
fn randomised_request_envelopes_match_live_python() {
    let corpus = random_request_payloads()
        .into_iter()
        .map(|payload| object(vec![("payload", payload)]))
        .collect::<Vec<_>>();
    let oracle = run_oracle("request", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entry) = case else {
            panic!("case is not an object")
        };
        let payload = entry.get(&JsonText::new("payload")).expect("payload");
        let native = request_case(payload);
        assert_same(&native, &expected, "randomised request corpus case");
    }
}

#[test]
fn randomised_response_envelopes_match_live_python() {
    let corpus = random_response_payloads()
        .into_iter()
        .map(|payload| object(vec![("payload", payload)]))
        .collect::<Vec<_>>();
    let oracle = run_oracle("response", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entry) = case else {
            panic!("case is not an object")
        };
        let payload = entry.get(&JsonText::new("payload")).expect("payload");
        let native = response_case(payload);
        assert_same(&native, &expected, "randomised response corpus case");
    }
}

#[test]
fn response_envelopes_match_live_python() {
    let corpus = response_payloads()
        .into_iter()
        .map(|payload| object(vec![("payload", payload)]))
        .collect::<Vec<_>>();
    let oracle = run_oracle("response", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entry) = case else {
            panic!("case is not an object")
        };
        let payload = entry.get(&JsonText::new("payload")).expect("payload");
        let native = response_case(payload);
        assert_same(&native, &expected, "response corpus case");
    }
}

// ---------------------------------------------------------------------------
// Workspace paths.
// ---------------------------------------------------------------------------

#[test]
fn workspace_paths_match_live_python() {
    let root = std::env::temp_dir().join("kala-tools-differential-workspace");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("outputs")).expect("create workspace");
    std::fs::create_dir_all(root.join("real")).expect("create workspace");
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).expect("create symlink");

    let long = "a".repeat(2_000);
    let relatives = vec![
        "a",
        "outputs/report.json",
        "./outputs/./report.json",
        "outputs",
        "real",
        "   ",
        "/etc/passwd",
        "a\x00b",
        "a\\b",
        "../escape",
        "a/../b",
        "..",
        ".",
        "./",
        "has space",
        long.as_str(),
        "link/report.json",
        "link",
        "a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/bb",
    ];
    let owned: Vec<String> = relatives.iter().map(|value| (*value).to_owned()).collect();
    let corpus = owned
        .iter()
        .map(|relative| object(vec![("relative", text(relative))]))
        .collect::<Vec<_>>();
    let oracle = run_oracle("workspace", &cases(corpus.clone()), Some(&root));
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entry) = case else {
            panic!("case is not an object")
        };
        let CanonicalValue::Text(relative) =
            entry.get(&JsonText::new("relative")).expect("relative")
        else {
            panic!("case has no relative path")
        };
        let relative = relative.to_string_lossy();
        let native = match resolve_workspace_path(&root, &relative) {
            Ok(path) => ok_result(text(&path.to_string_lossy().replace('\\', "/"))),
            Err(error) => error_result(&error),
        };
        assert_same(&native, &expected, "workspace corpus case");
    }
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Child diagnostics.
// ---------------------------------------------------------------------------

#[test]
fn child_diagnostics_match_live_python() {
    let mut payloads: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"first\nsecond".to_vec(),
        b"Traceback (most recent call last):\n  File x\nValueError: boom".to_vec(),
        b"first\nsecond\nthird\nfourth\nfifth".to_vec(),
        b"\x00\x01only".to_vec(),
        b"a\r\nb".to_vec(),
        b"  \n \n ".to_vec(),
        b"failed in /tmp/ws-1234".to_vec(),
        vec![b'x'; 4_000],
        b"line one\nline two\nline three".to_vec(),
        b"tab\there".to_vec(),
    ];
    payloads.push((0..=255u8).collect::<Vec<u8>>());
    let mut corpus = Vec::new();
    for payload in &payloads {
        corpus.push(object(vec![("hex", text(&hex(payload)))]));
        corpus.push(object(vec![
            ("hex", text(&hex(payload))),
            ("replacement", text("/tmp/ws")),
        ]));
    }
    let oracle = run_oracle("diagnostics", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entry) = case else {
            panic!("case is not an object")
        };
        if !matches!(
            entry.get(&JsonText::new("hex")),
            Some(CanonicalValue::Text(_))
        ) {
            panic!("case has no hex")
        }
        let replacement = entry
            .get(&JsonText::new("replacement"))
            .and_then(|value| value.as_text())
            .map(|value| value.to_string_lossy());
        let payload = hex_decode(case);
        let native = ok_result(text(&sanitize_child_stderr(
            &payload,
            replacement.as_deref(),
        )));
        assert_same(&native, &expected, "diagnostics corpus case");
    }
}

// ---------------------------------------------------------------------------
// Error summaries.
// ---------------------------------------------------------------------------

#[test]
fn error_summaries_match_live_python() {
    let mut corpus = Vec::new();
    for kind in ["protocol", "json", "envelope", "workspace", "schema"] {
        for message in [
            "",
            "   ",
            "boom",
            "boom boom",
            "at /tmp/ws/x",
            "line one\nline two",
            "tabs\there",
        ] {
            let mut entries = vec![("kind", text(kind)), ("message", text(message))];
            if kind == "schema" {
                entries.push(("path", text("$.a")));
            }
            corpus.push(object(entries));
        }
    }
    let mut with_replacement = Vec::new();
    for case in &corpus {
        let CanonicalValue::Object(entries) = case else {
            panic!("case is not an object")
        };
        let mut entries = entries.clone();
        entries.insert(JsonText::new("replacement"), text("/tmp/ws"));
        with_replacement.push(CanonicalValue::Object(entries));
    }
    corpus.extend(with_replacement);

    let oracle = run_oracle("summary", &cases(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (case, expected) in corpus.iter().zip(oracle) {
        let CanonicalValue::Object(entries) = case else {
            panic!("case is not an object")
        };
        let kind = entries
            .get(&JsonText::new("kind"))
            .and_then(|value| value.as_text())
            .expect("kind")
            .to_string_lossy();
        let message = entries
            .get(&JsonText::new("message"))
            .and_then(|value| value.as_text())
            .expect("message")
            .to_string_lossy();
        let path = entries
            .get(&JsonText::new("path"))
            .and_then(|value| value.as_text())
            .map(|value| value.to_string_lossy());
        let replacement = entries
            .get(&JsonText::new("replacement"))
            .and_then(|value| value.as_text())
            .map(|value| value.to_string_lossy());
        let error = match kind.as_str() {
            "protocol" => kala_tools::protocol::ToolProtocolError::protocol(&message),
            "json" => kala_tools::protocol::ToolProtocolError::json_envelope(&message),
            "envelope" => kala_tools::protocol::ToolProtocolError::envelope(&message),
            "workspace" => kala_tools::protocol::ToolProtocolError::workspace(&message),
            _ => kala_tools::protocol::ToolProtocolError::schema(
                &message,
                path.unwrap_or_else(|| "$".to_owned()),
            ),
        };
        let native = ok_result(text(&summarize_error(&error, replacement.as_deref())));
        assert_same(&native, &expected, "summary corpus case");
    }
}

// ---------------------------------------------------------------------------
// The corpus itself must stay inside the protocol limits.
// ---------------------------------------------------------------------------

#[test]
fn corpus_documents_are_canonical() {
    for corpus in [
        parse_corpus(),
        encode_corpus(),
        validate_corpus(),
        check_corpus(),
    ] {
        let encoded = canonical(&cases(corpus));
        assert!(encoded.len() < 32 * 1024 * 1024, "corpus is too large");
    }
}

/// The size of every differential corpus.
///
/// The numbers are asserted so the report cannot drift away from the corpus
/// that actually runs.
#[test]
fn the_differential_corpus_sizes_are_stable() {
    assert_eq!(parse_corpus().len(), 48);
    assert_eq!(encode_corpus().len(), 141);
    assert_eq!(validate_corpus().len(), 288);
    assert_eq!(check_corpus().len(), 14);
    assert_eq!(request_payloads().len(), 28);
    assert_eq!(response_payloads().len(), 20);
    assert_eq!(random_request_payloads().len(), 200);
    assert_eq!(random_response_payloads().len(), 200);
}
