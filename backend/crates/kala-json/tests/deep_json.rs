//! Stack-safety coverage for deeply nested canonical JSON.
//!
//! The tool-protocol domain stays bounded at depth 48, but the persistence
//! `CanonicalJsonObject` domain is effectively unbounded. These tests run
//! every operation that touches a deep enum value (parse, validate, encode,
//! clone, equality, debug and drop) on an ordinary 2 MiB thread stack, so a
//! hidden recursive implementation overflows here rather than in production.
//! They also check malformed deep input and wide containers so the heap-stack
//! implementation is exercised for error cleanup and linear-width behaviour.

use std::time::Instant;

use kala_json::{CanonicalJsonError, CanonicalValue, Limits, canonical_dumps, parse_strict};

fn deep_limits() -> Limits {
    Limits {
        max_json_bytes: usize::MAX,
        max_string_length: usize::MAX,
        max_collection_items: usize::MAX,
        max_key_length: usize::MAX,
        max_json_depth: usize::MAX,
        max_integer_digits: Limits::DEFAULT_MAX_INTEGER_DIGITS,
    }
}

fn on_two_mib_stack<F, T>(operation: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(operation)
        .expect("spawn 2 MiB test thread")
        .join()
        .expect("operation must not overflow the 2 MiB stack")
}

fn nested_array_source(depth: usize) -> String {
    let mut source = String::with_capacity(depth * 2 + 1);
    for _ in 0..depth {
        source.push('[');
    }
    source.push('0');
    for _ in 0..depth {
        source.push(']');
    }
    source
}

fn nested_array_value(depth: usize) -> CanonicalValue {
    let mut value = CanonicalValue::integer(0);
    for _ in 0..depth {
        value = CanonicalValue::Array(vec![value]);
    }
    value
}

/// Alternating arrays/objects around a mixed leaf. The top level is an object
/// and all keys are inserted in canonical order.
fn mixed_source(depth: usize) -> String {
    let mut prefix = String::from(r#"{"x":"#);
    let mut closers = Vec::with_capacity(depth);
    for level in 0..depth {
        if level % 2 == 0 {
            prefix.push('[');
            closers.push(']');
        } else {
            prefix.push_str(r#"{"k":"#);
            closers.push('}');
        }
    }
    prefix
        .push_str(r#"{"big":1180591620717411303424,"f":-0.0,"n":null,"u":"\u03ba \ud83d\ude00"}"#);
    for closer in closers.into_iter().rev() {
        prefix.push(closer);
    }
    prefix.push('}');
    prefix
}

fn mixed_value(depth: usize) -> CanonicalValue {
    let mut value = CanonicalValue::object([
        ("n", CanonicalValue::Null),
        (
            "u",
            CanonicalValue::text(
                kala_json::JsonText::from_code_points(vec![0x03BA, 0x20, 0x1F600])
                    .expect("valid code points"),
            ),
        ),
        (
            "big",
            CanonicalValue::Integer("1180591620717411303424".parse().expect("big integer")),
        ),
        ("f", CanonicalValue::Float(-0.0)),
    ]);
    for level in 0..depth {
        if level % 2 == 0 {
            value = CanonicalValue::Array(vec![value]);
        } else {
            value = CanonicalValue::object([("k", value)]);
        }
    }
    CanonicalValue::object([("x", value)])
}

#[test]
fn parse_dump_clone_eq_debug_and_drop_are_stack_safe() {
    on_two_mib_stack(|| {
        for depth in [1_500usize, 5_000, 10_000, 20_000, 50_000] {
            let source = mixed_source(depth);
            let value = parse_strict(source.as_bytes(), &deep_limits())
                .unwrap_or_else(|error| panic!("parse depth {depth}: {error}"));
            let canonical = canonical_dumps(&value, &deep_limits())
                .unwrap_or_else(|error| panic!("dump depth {depth}: {error}"));
            assert_eq!(canonical, source, "canonical bytes at depth {depth}");

            let reparsed = parse_strict(canonical.as_bytes(), &deep_limits())
                .unwrap_or_else(|error| panic!("reparse depth {depth}: {error}"));
            assert_eq!(reparsed, value, "equality at depth {depth}");

            let cloned = value.clone();
            assert_eq!(cloned, value, "clone equality at depth {depth}");
            assert_eq!(
                canonical_dumps(&cloned, &deep_limits()).expect("dump clone"),
                canonical,
                "clone bytes at depth {depth}"
            );

            let debug = format!("{value:?}");
            assert!(
                debug.starts_with(r#"Object({JsonText("x"): "#),
                "debug shape at depth {depth}"
            );
            assert!(
                debug.len() > depth,
                "debug output should be non-empty at depth {depth}"
            );

            // Explicit drops: value, clone, reparsed and the debug String all
            // leave scope on the 2 MiB stack while still nested.
            drop(value);
            drop(cloned);
            drop(reparsed);
            drop(debug);
        }
    });
}

#[test]
fn typed_values_clone_eq_debug_and_drop_are_stack_safe() {
    on_two_mib_stack(|| {
        for depth in [5_000usize, 20_000, 50_000] {
            let value = mixed_value(depth);
            let canonical = canonical_dumps(&value, &deep_limits()).expect("dump typed value");
            let cloned = value.clone();
            assert_eq!(value, cloned);
            let debug = format!("{value:?}");
            assert!(!debug.is_empty());
            drop(value);
            drop(cloned);
            drop(debug);
            // Rebuild a deep typed value from the canonical bytes and drop it.
            let reparsed = parse_strict(canonical.as_bytes(), &deep_limits()).expect("reparse");
            drop(reparsed);
        }
    });
}

#[test]
fn tool_depth_limit_stays_48_even_for_deep_syntax() {
    on_two_mib_stack(|| {
        let at_limit = nested_array_source(48);
        assert!(
            parse_strict(at_limit.as_bytes(), &Limits::default()).is_ok(),
            "depth 48 must remain accepted"
        );

        for depth in [49usize, 1_500, 20_000] {
            let source = nested_array_source(depth);
            assert_eq!(
                parse_strict(source.as_bytes(), &Limits::default()),
                Err(CanonicalJsonError::DepthExceeded),
                "tool depth {depth}"
            );
        }

        // A very deep syntactically incomplete document must fail closed, not
        // recurse or leak a partially built value. Use exactly the tool byte
        // cap so the heap parser stack is exercised at the maximum input size.
        let unterminated = "[".repeat(Limits::DEFAULT_MAX_JSON_BYTES);
        assert_eq!(
            parse_strict(unterminated.as_bytes(), &Limits::default()),
            Err(CanonicalJsonError::NotJson)
        );
        let over_cap = "[".repeat(Limits::DEFAULT_MAX_JSON_BYTES + 1);
        assert_eq!(
            parse_strict(over_cap.as_bytes(), &Limits::default()),
            Err(CanonicalJsonError::InputTooLarge)
        );
    });
}

#[test]
fn deep_malformed_inputs_clean_up_without_overflow() {
    on_two_mib_stack(|| {
        let mut powers = String::new();
        for _ in 0..20_000 {
            powers.push_str(r#"{"a":"#);
        }
        powers.push('0');
        let cases = [
            // Missing every closing brace.
            powers,
            // Complete but with trailing data.
            format!("{}0{}x", "[".repeat(20_000), "]".repeat(20_000)),
            // Unterminated string inside a deep array.
            format!("{} \"unterminated", "[".repeat(20_000)),
            // Bad number shape inside a deep array.
            format!("{}01{}", "[".repeat(20_000), "]".repeat(20_000)),
            // Missing comma between deep array items.
            format!("{}0 1{}", "[".repeat(20_000), "]".repeat(20_000)),
            // Duplicate key after a completed deep value.
            format!(
                r#"{{"a":{}0{},"a":1}}"#,
                "[".repeat(20_000),
                "]".repeat(20_000)
            ),
        ];
        for (index, source) in cases.iter().enumerate() {
            assert!(
                parse_strict(source.as_bytes(), &deep_limits()).is_err(),
                "malformed deep case {index} must be rejected"
            );
        }
    });
}

#[test]
fn wide_containers_are_handled_without_quadratic_rebuilding() {
    on_two_mib_stack(|| {
        let width = 200_000usize;
        let source = format!(
            "[{}]",
            std::iter::repeat_n("0", width)
                .collect::<Vec<_>>()
                .join(",")
        );
        let started = Instant::now();
        let value = parse_strict(source.as_bytes(), &deep_limits()).expect("parse wide array");
        let CanonicalValue::Array(items) = &value else {
            panic!("expected an array");
        };
        assert_eq!(items.len(), width);
        let canonical = canonical_dumps(&value, &deep_limits()).expect("dump wide array");
        assert_eq!(canonical.len(), source.len());
        let cloned = value.clone();
        assert_eq!(cloned, value);
        drop(value);
        drop(cloned);
        assert!(
            started.elapsed().as_secs() < 30,
            "200k-wide container took {:?}; likely quadratic",
            started.elapsed()
        );

        // A wide object exercises BTreeMap building, sorted-key encoding,
        // cloning and iterative drop as well.
        let object_width = 20_000usize;
        let object_source = format!(
            "{{{}}}",
            (0..object_width)
                .map(|index| format!(r#""k{index}":null"#))
                .collect::<Vec<_>>()
                .join(",")
        );
        let object = parse_strict(object_source.as_bytes(), &deep_limits()).expect("wide object");
        let object_canonical = canonical_dumps(&object, &deep_limits()).expect("dump object");
        let object_clone = object.clone();
        assert_eq!(object, object_clone);
        assert_eq!(
            parse_strict(object_canonical.as_bytes(), &deep_limits()).expect("reparse object"),
            object
        );
        drop(object);
        drop(object_clone);
    });
}

#[test]
fn debug_text_matches_the_derived_shape_for_shallow_values() {
    let value = CanonicalValue::object([(
        "a",
        CanonicalValue::array([
            CanonicalValue::Null,
            CanonicalValue::Bool(true),
            CanonicalValue::integer(2),
            CanonicalValue::Float(1.5),
            CanonicalValue::text("x"),
        ]),
    )]);
    assert_eq!(
        format!("{value:?}"),
        concat!(
            r#"Object({JsonText("a"): Array([Null, Bool(true), Integer(2), "#,
            r#"Float(1.5), Text(JsonText("x"))])})"#
        )
    );
}

#[test]
fn nested_array_values_round_trip_at_every_required_depth() {
    on_two_mib_stack(|| {
        for depth in [1_500usize, 5_000, 10_000, 20_000] {
            let source = nested_array_source(depth);
            let parsed = parse_strict(source.as_bytes(), &deep_limits()).expect("parse arrays");
            assert_eq!(parsed, nested_array_value(depth));
            assert_eq!(
                canonical_dumps(&parsed, &deep_limits()).expect("dump arrays"),
                source
            );
        }
    });
}
