//! Scalar normalization, scale and range tests.

use kala_contracts::{
    ArtifactUri, FiniteNonNegativeFloat, Identifier, MediaType, NonNegativeDecimal,
    NonNegativeInteger, Sha256Digest, Text, Timestamp, UnitInterval,
};

#[test]
fn identifiers_use_python_strip_and_ascii_regex() {
    assert_eq!(
        Identifier::new("\u{1c} req-1 \u{1f}").unwrap().as_str(),
        "req-1"
    );
    assert_eq!(
        Identifier::new(&"a".repeat(128)).unwrap().as_str().len(),
        128
    );
    assert!(Identifier::new(&"a".repeat(129)).is_err());
    assert!(Identifier::new("é").is_err());
    assert!(Identifier::new("a b").is_err());
}

#[test]
fn media_types_and_digests_normalize_lowercase() {
    assert_eq!(
        MediaType::new(" APPLICATION/VND.X+JSON ").unwrap().as_str(),
        "application/vnd.x+json"
    );
    assert!(MediaType::new("text/plain; charset=utf-8").is_err());
    let digest = Sha256Digest::from_hex(&"A".repeat(64)).unwrap();
    assert_eq!(digest.to_hex(), "a".repeat(64));
}

#[test]
fn artifact_uris_normalize_hosts_paths_and_ports() {
    let cases = [
        (
            "HTTPS://Example.COM:443/objects//item%20one",
            "https://example.com/objects/item%20one",
        ),
        ("file://localhost:123/tmp//a/", "file:///tmp/a"),
        ("artifact://Objects/foo", "artifact://objects/foo"),
        (
            "https://bücher.example/a",
            "https://xn--bcher-kva.example/a",
        ),
        ("https://faß.de/a", "https://fass.de/a"),
        ("https://[2001:DB8::1]:443/foo", "https://[2001:db8::1]/foo"),
        (
            "https://[::ffff:c000:201]/a",
            "https://[::ffff:192.0.2.1]/a",
        ),
        (
            "https://[2001:db8::1:0:0:0]/a",
            "https://[2001:db8:0:0:1::]/a",
        ),
        ("https://example.com/a//b", "https://example.com/a/b"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            ArtifactUri::parse(input).unwrap().as_text().as_str(),
            Some(expected),
            "{input}"
        );
    }
}

#[test]
fn artifact_uris_reject_security_cases() {
    for input in [
        "relative/path",
        "javascript:alert(1)",
        "https://user:password@example.com/object",
        "https://example.com/object?token=secret",
        "https://example.com/object#fragment",
        "file://remote-host/object",
        "https://example.com/a/%2e%2e/b",
        "https://example.com/a/%00",
        "https://example.com/a\\b",
        "https://example.com/%ff",
        "https://example.com:65536/a",
        "https://example.com:notaport/a",
        "https://[::1]x/a",
        "https://example.com/a/./b",
        "https://example.com/..",
        "https:///a",
        "artifact:/a",
    ] {
        assert!(ArtifactUri::parse(input).is_err(), "{input}");
    }
}

#[test]
fn nonnegative_integers_keep_arbitrary_precision() {
    let value = "9".repeat(300);
    let number = NonNegativeInteger::new(value.parse().unwrap()).unwrap();
    assert_eq!(number.as_bigint().to_string(), value);
    assert!(NonNegativeInteger::new("-1".parse().unwrap()).is_err());
    assert_eq!(
        NonNegativeInteger::from_u64(2_u64.pow(63)).to_u64(),
        Some(1 << 63)
    );
}

#[test]
fn decimals_preserve_textual_scale_and_signed_zero() {
    assert_eq!(
        NonNegativeDecimal::parse("1.00")
            .unwrap()
            .to_python_string(),
        "1.00"
    );
    assert_eq!(
        NonNegativeDecimal::parse("1E+2")
            .unwrap()
            .to_python_string(),
        "1E+2"
    );
    assert_eq!(
        NonNegativeDecimal::parse("-0.00")
            .unwrap()
            .to_python_string(),
        "-0.00"
    );
    assert!(NonNegativeDecimal::parse("-1").is_err());
    assert!(NonNegativeDecimal::parse("NaN").is_err());
}

#[test]
fn floats_accept_negative_zero_and_reject_nonfinite() {
    assert_eq!(
        FiniteNonNegativeFloat::new(-0.0).unwrap().get().to_bits(),
        (-0.0f64).to_bits()
    );
    assert!(FiniteNonNegativeFloat::new(-1.0).is_err());
    assert!(FiniteNonNegativeFloat::new(f64::NAN).is_err());
    assert!(FiniteNonNegativeFloat::new(f64::INFINITY).is_err());
    assert_eq!(UnitInterval::new(0.0).unwrap().get(), 0.0);
    assert_eq!(UnitInterval::new(1.0).unwrap().get(), 1.0);
    assert!(UnitInterval::new(1.0000001).is_err());
}

#[test]
fn timestamps_preserve_offsets_and_compare_instants() {
    let utc = Timestamp::parse("2026-01-02T03:04:05Z").unwrap();
    let offset = Timestamp::parse("2026-01-02T08:34:05+05:30").unwrap();
    assert_eq!(utc, offset);
    assert_eq!(offset.to_wire_string(), "2026-01-02T08:34:05+05:30");
    assert_eq!(
        Timestamp::parse("2026-01-02T03:04:05.123456789Z")
            .unwrap()
            .to_wire_string(),
        "2026-01-02T03:04:05.123456Z"
    );
    assert!(Timestamp::parse("2026-01-02T03:04:05").is_err());
    assert!(Timestamp::parse("2026-02-29T03:04:05Z").is_err());
    assert_eq!(Text::new("a").len(), 1);
}
